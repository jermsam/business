//! External payment calls. Application data never includes PAN/CVC or raw card forms.
use super::billing_schema::{Mode, Provider};
use anyhow::{ensure, Result};
use reqwest::{Client, Method};
use serde_json::{json, Value};
#[derive(Clone)]
pub struct Providers {
    http: Client,
    pub live: bool,
}
impl Providers {
    pub fn new() -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            http: Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(std::time::Duration::from_secs(25))
                .build()?,
            live: std::env::var("BILLING_LIVE").as_deref() == Ok("true"),
        })
    }
    pub async fn stripe(
        &self,
        method: Method,
        path: &str,
        key: &str,
        form: &[(String, String)],
    ) -> Result<Value> {
        ensure!(
            path.starts_with('/') && !path.contains(['?', '#', '\r', '\n']),
            "Invalid provider path"
        );
        let token = std::env::var("STRIPE_SECRET_KEY")?;
        ensure!(
            if self.live {
                token.starts_with("sk_live_") || token.starts_with("rk_live_")
            } else {
                token.starts_with("sk_test_") || token.starts_with("rk_test_")
            },
            "Stripe key does not match billing mode"
        );
        let mut url = reqwest::Url::parse(&format!("https://api.stripe.com/v1{path}"))?;
        if method == Method::GET && !form.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(form.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        }
        let mut request = self
            .http
            .request(method.clone(), url)
            .bearer_auth(token)
            .header("Stripe-Version", "2026-03-25.dahlia");
        if method != Method::GET {
            request = request.header("Idempotency-Key", key).form(form);
        }
        let response = request.send().await?;
        ensure!(
            response.status().is_success(),
            "Stripe request failed with HTTP {}; reconcile before retrying",
            response.status()
        );
        let result: Value = response.json().await?;
        if let Some(mode) = result["livemode"].as_bool() {
            ensure!(mode == self.live, "Provider mode mismatch");
        }
        Ok(result)
    }
    pub async fn mercury(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        ensure!(
            path.starts_with('/') && !path.contains(['#', '\r', '\n']),
            "Invalid provider path"
        );
        let base = if self.live {
            "https://api.mercury.com/api/v1"
        } else {
            "https://api-sandbox.mercury.com/api/v1"
        };
        let key = std::env::var(if self.live {
            "MERCURY_API_TOKEN"
        } else {
            "MERCURY_SANDBOX_TOKEN"
        })?;
        let mut request = self
            .http
            .request(method, format!("{base}{path}"))
            .bearer_auth(key);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        ensure!(
            response.status().is_success(),
            "Mercury request failed with HTTP {}; reconcile before retrying",
            response.status()
        );
        Ok(response.json().await?)
    }
    pub async fn customer(
        &self,
        provider: Provider,
        id: &str,
        name: &str,
        email: &str,
    ) -> Result<String> {
        let result = match provider {
            Provider::Mercury => {
                self.mercury(
                    Method::POST,
                    "/ar/customers",
                    Some(json!({"name":name,"email":email})),
                )
                .await?
            }
            Provider::Stripe => {
                self.stripe(
                    Method::POST,
                    "/customers",
                    &format!("business-customer-{id}"),
                    &pairs(&[
                        ("name", name),
                        ("email", email),
                        ("metadata[business_customer]", id),
                    ]),
                )
                .await?
            }
        };
        Ok(field(&result, "id")?.to_owned())
    }
    pub async fn setup(
        &self,
        customer: &str,
        id: &str,
        revision: &str,
        actor: &str,
    ) -> Result<Value> {
        let base = std::env::var("BILLING_PUBLIC_URL")?;
        let parsed = reqwest::Url::parse(&base)?;
        ensure!(
            parsed.scheme() == "https"
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.query().is_none()
                && parsed.fragment().is_none(),
            "Configure a trusted HTTPS billing return URL"
        );
        let form=pairs(&[("mode","setup"),("customer",customer),("payment_method_types[0]","card"),("success_url",&format!("{base}?customer={id}&setup={{CHECKOUT_SESSION_ID}}")),("cancel_url",&base),("client_reference_id",id),("setup_intent_data[metadata][business_customer]",id),("setup_intent_data[metadata][business_revision]",revision),("setup_intent_data[metadata][business_actor]",actor),("custom_text[submit][message]","By saving this card, you authorize automatic payment of the billing schedules you reviewed. You can disable automatic payments in your billing page."),("consent_collection[payment_method_reuse_agreement][position]","auto")]);
        self.stripe(
            Method::POST,
            "/checkout/sessions",
            &format!("setup-{id}-{revision}-{actor}"),
            &form,
        )
        .await
    }
    pub async fn invoice(&self, input: &InvoiceRequest) -> Result<Value> {
        match input.provider {
            Provider::Mercury => {
                ensure!(
                    input.mode == Mode::Manual,
                    "Mercury API does not implement saved-card autopay"
                );
                let destination = std::env::var(if self.live {
                    "MERCURY_ACCOUNT_ID"
                } else {
                    "MERCURY_SANDBOX_ACCOUNT_ID"
                })?;
                uuid::Uuid::parse_str(&destination)?;
                // Serialize a decimal JSON number from integer cents; do not calculate money in f64.
                let amount: serde_json::Number =
                    format!("{}.{:02}", input.amount / 100, input.amount % 100).parse()?;
                self.mercury(Method::POST,"/ar/invoices",Some(json!({"customerId":input.customer,"invoiceNumber":format!("JIT-{}",input.id),"destinationAccountId":destination,"currencyCode":"USD","invoiceDate":date(input.issued)?,"dueDate":date(input.due)?,"ccEmails":[],"achDebitEnabled":false,"creditCardEnabled":true,"useRealAccountNumber":false,"sendEmailOption":"SendNow","lineItems":[{"name":input.description,"quantity":1,"unitPrice":amount}]}))).await
            }
            Provider::Stripe => {
                // Keep advancement disabled. Business checks current consent at the due date
                // and explicitly attempts payment once; Stripe never schedules a hidden retry.
                let form = pairs(&[
                    ("customer", &input.customer),
                    ("collection_method", "send_invoice"),
                    ("auto_advance", "false"),
                    ("pending_invoice_items_behavior", "exclude"),
                    ("currency", "usd"),
                    (
                        "due_date",
                        &input
                            .due
                            .max(chrono::Utc::now().timestamp() + 60)
                            .to_string(),
                    ),
                    ("metadata[business_invoice]", &input.id),
                ]);
                self.stripe(
                    Method::POST,
                    "/invoices",
                    &format!("invoice-{}", input.id),
                    &form,
                )
                .await
            }
        }
    }
    pub async fn finish_invoice(&self, input: &InvoiceRequest, id: &str) -> Result<Value> {
        safe_id(id)?;
        self.stripe(
            Method::POST,
            "/invoiceitems",
            &format!("item-{}", input.id),
            &pairs(&[
                ("customer", &input.customer),
                ("invoice", id),
                ("amount", &input.amount.to_string()),
                ("currency", "usd"),
                ("description", &input.description),
            ]),
        )
        .await?;
        self.stripe(
            Method::POST,
            &format!("/invoices/{id}/finalize"),
            &format!("finalize-{}", input.id),
            &pairs(&[("auto_advance", "false")]),
        )
        .await
    }
    pub async fn charge(&self, id: &str, local: &str, card: &str) -> Result<Value> {
        safe_id(id)?;
        safe_id(card)?;
        self.stripe(
            Method::POST,
            &format!("/invoices/{id}/pay"),
            &format!("charge-{local}"),
            &pairs(&[("payment_method", card), ("off_session", "true")]),
        )
        .await
    }
    pub async fn email(
        &self,
        token: &str,
        from: &str,
        to: &str,
        subject: &str,
        body: &str,
        key: &str,
    ) -> Result<String> {
        let response = self
            .http
            .post("https://api.resend.com/emails")
            .bearer_auth(token)
            .header("Idempotency-Key", key)
            .json(&json!({"from":from,"to":[to],"subject":subject,"text":body}))
            .send()
            .await?;
        ensure!(
            response.status().is_success(),
            "Email send outcome requires reconciliation"
        );
        let value: Value = response.json().await?;
        Ok(field(&value, "id")?.into())
    }
    pub async fn observe(&self, provider: Provider, id: &str) -> Result<Value> {
        safe_id(id)?;
        match provider {
            Provider::Mercury => {
                self.mercury(Method::GET, &format!("/ar/invoices/{id}"), None)
                    .await
            }
            Provider::Stripe => {
                self.stripe(Method::GET, &format!("/invoices/{id}"), "", &[])
                    .await
            }
        }
    }
}
pub struct InvoiceRequest {
    pub id: String,
    pub customer: String,
    pub provider: Provider,
    pub mode: Mode,
    pub amount: i64,
    pub description: String,
    pub issued: i64,
    pub due: i64,
}
pub fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Provider omitted required field {key}"))
}
pub fn safe_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() < 200
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "Invalid provider ID"
    );
    Ok(())
}
fn date(at: i64) -> Result<String> {
    Ok(chrono::DateTime::from_timestamp(at, 0)
        .ok_or_else(|| anyhow::anyhow!("Invalid timestamp"))?
        .format("%Y-%m-%d")
        .to_string())
}
pub fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
pub fn status(provider: Provider, value: &Value) -> Result<&'static str> {
    Ok(match (provider, field(value, "status")?) {
        (Provider::Mercury, "Paid") | (Provider::Stripe, "paid") => "paid",
        (Provider::Mercury, "Cancelled") | (Provider::Stripe, "void") => "void",
        (Provider::Mercury, "Processing") => "processing",
        (Provider::Stripe, "draft") => "scheduled",
        (Provider::Stripe, "uncollectible") => "uncollectible",
        (Provider::Mercury, "Unpaid") | (Provider::Stripe, "open") => "open",
        _ => anyhow::bail!("Unrecognized invoice state"),
    })
}
pub fn pay_url(provider: Provider, value: &Value, live: bool) -> Result<String> {
    match provider {
        Provider::Stripe => {
            let url = value["hosted_invoice_url"].as_str().unwrap_or("");
            if !url.is_empty() {
                trusted_url(url, "invoice.stripe.com")?;
            }
            Ok(url.into())
        }
        Provider::Mercury => {
            let slug = field(value, "slug")?;
            safe_id(slug)?;
            Ok(format!(
                "https://{}/pay/{slug}",
                if live {
                    "app.mercury.com"
                } else {
                    "sandbox.mercury.com"
                }
            ))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unexpected_states_and_paths_fail_closed() {
        assert!(safe_id("../invoices").is_err());
        assert!(status(Provider::Mercury, &json!({"status":"SomethingNew"})).is_err());
        assert_eq!(
            status(Provider::Mercury, &json!({"status":"Processing"})).unwrap(),
            "processing"
        );
        assert!(pay_url(
            Provider::Stripe,
            &json!({"hosted_invoice_url":"https://evil.example/pay"}),
            false
        )
        .is_err());
    }
}

pub fn trusted_url(url: &str, host: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url)?;
    ensure!(
        parsed.scheme() == "https"
            && parsed.host_str() == Some(host)
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.port().is_none(),
        "Unexpected payment URL"
    );
    Ok(())
}
/// Exact cents comparison, rejecting negative, fractional-cent and exponent input.
pub fn decimal_cents(value: &Value) -> Result<i64> {
    let raw = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let (whole, fraction) = raw.split_once('.').unwrap_or((&raw, ""));
    ensure!(
        !whole.is_empty()
            && whole.bytes().all(|c| c.is_ascii_digit())
            && fraction.len() <= 2
            && fraction.bytes().all(|c| c.is_ascii_digit()),
        "Invalid USD amount"
    );
    let cents = whole
        .parse::<i64>()?
        .checked_mul(100)
        .ok_or_else(|| anyhow::anyhow!("Amount overflow"))?;
    cents
        .checked_add(if fraction.is_empty() {
            0
        } else {
            fraction.parse::<i64>()? * if fraction.len() == 1 { 10 } else { 1 }
        })
        .ok_or_else(|| anyhow::anyhow!("Amount overflow"))
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Requires explicitly approved local Stripe sandbox key"]
    async fn stripe_sandbox_invoice_lifecycle() {
        dotenvy::from_path(
            std::env::var("BILLING_TEST_ENV")
                .expect("Set BILLING_TEST_ENV to the protected sandbox configuration"),
        )
        .unwrap();
        let providers = Providers::new().unwrap();
        assert!(!providers.live, "Never charge real money in tests");
        let id = uuid::Uuid::new_v4().to_string();
        let customer = providers
            .customer(
                Provider::Stripe,
                &id,
                "Business sandbox validation",
                "billing-validation@example.com",
            )
            .await
            .unwrap();
        let checkout = providers
            .setup(&customer, &id, "test-revision", "test-actor")
            .await
            .unwrap();
        assert_eq!(checkout["mode"], "setup");
        trusted_url(field(&checkout, "url").unwrap(), "checkout.stripe.com").unwrap();
        let setup = providers
            .stripe(
                Method::POST,
                "/setup_intents",
                &format!("test-setup-{id}"),
                &pairs(&[
                    ("customer", &customer),
                    ("payment_method", "pm_card_visa"),
                    ("payment_method_types[0]", "card"),
                    ("usage", "off_session"),
                    ("confirm", "true"),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(setup["status"], "succeeded");
        let card = field(&setup, "payment_method").unwrap();
        let input = InvoiceRequest {
            id: id.clone(),
            customer: customer.clone(),
            provider: Provider::Stripe,
            mode: Mode::Automatic,
            amount: 1250,
            description: "Sandbox test; no real goods or charge".into(),
            issued: chrono::Utc::now().timestamp(),
            due: chrono::Utc::now().timestamp() + 86400,
        };
        let draft = providers.invoice(&input).await.unwrap();
        assert_eq!(draft["status"], "draft");
        assert_eq!(draft["auto_advance"], false);
        let external = field(&draft, "id").unwrap();
        let open = providers.finish_invoice(&input, external).await.unwrap();
        assert_eq!(open["status"], "open");
        assert_eq!(open["total"], 1250);
        assert_eq!(open["auto_advance"], false);
        assert!(!pay_url(Provider::Stripe, &open, false).unwrap().is_empty());
        let paid = providers.charge(external, &id, card).await.unwrap();
        assert_eq!(paid["status"], "paid");
        assert_eq!(paid["amount_paid"], 1250);
        let observed = providers.observe(Provider::Stripe, external).await.unwrap();
        assert_eq!(observed["status"], "paid");
        // Replaying the same provider idempotency key returns the same invoice.
        let again = providers.charge(external, &id, card).await.unwrap();
        assert_eq!(again["id"], paid["id"]);
    }
}
#[cfg(test)]
mod mercury_live_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Requires approved IP-restricted Mercury sandbox token"]
    async fn mercury_sandbox_invoice_lifecycle() {
        dotenvy::from_path(
            std::env::var("BILLING_TEST_ENV")
                .expect("Set BILLING_TEST_ENV to the protected sandbox configuration"),
        )
        .unwrap();
        let provider = Providers::new().unwrap();
        assert!(!provider.live);
        let id = uuid::Uuid::new_v4().to_string();
        let customer = provider
            .customer(
                Provider::Mercury,
                &id,
                "Business sandbox validation",
                "billing-validation@example.com",
            )
            .await
            .unwrap();
        let input = InvoiceRequest {
            id: id.clone(),
            customer,
            provider: Provider::Mercury,
            mode: Mode::Manual,
            amount: 1250,
            description: "Sandbox validation, no real payment".into(),
            issued: chrono::Utc::now().timestamp(),
            due: chrono::Utc::now().timestamp() + 86400,
        };
        let created = provider.invoice(&input).await.unwrap();
        assert_eq!(created["invoiceNumber"], format!("JIT-{id}"));
        assert_eq!(decimal_cents(&created["amount"]).unwrap(), 1250);
        let invoice_id = field(&created, "id").unwrap();
        let read = provider
            .observe(Provider::Mercury, invoice_id)
            .await
            .unwrap();
        assert_eq!(read["id"], created["id"]);
        assert_eq!(status(Provider::Mercury, &read).unwrap(), "open");
    }
}

#[cfg(test)]
mod email_live_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Requires approved local Resend credential and test recipient"]
    async fn resend_test_delivery_and_idempotency() {
        dotenvy::from_path(
            std::env::var("BILLING_TEST_ENV").expect("Protected test configuration required"),
        )
        .unwrap();
        let providers = Providers::new().unwrap();
        assert!(!providers.live);
        let token = std::env::var("RESEND_API_KEY").unwrap();
        let from = std::env::var("BILLING_EMAIL_FROM").unwrap();
        let recipient = std::env::var("BILLING_TEST_EMAIL").unwrap();
        assert_eq!(
            recipient, "dev@jitpomi.com",
            "Only the approved test recipient"
        );
        let key = format!("business-email-validation-{}", uuid::Uuid::new_v4());
        let subject = "[TEST — no real payment] JITPOMI billing email validation";
        let body = "This is an authorized JITPOMI Business email integration test. No invoice is due, no payment was taken, and no action is required. This validates the email channel for future invoice notices, receipts and reminders. It is not a real receipt.";
        let first = providers
            .email(&token, &from, &recipient, subject, body, &key)
            .await
            .unwrap();
        let replay = providers
            .email(&token, &from, &recipient, subject, body, &key)
            .await
            .unwrap();
        assert_eq!(first, replay, "Same email must not be created twice");
        println!(
            "Resend accepted test email; idempotent replay returned the same message ID: {first}"
        );
    }
}
