//! External payment calls. Application data never includes PAN/CVC or raw card forms.
use super::billing_schema::{Mode, Provider};
use anyhow::{ensure, Result};
use reqwest::{Client, Method};
use serde_json::{json, Value};
#[derive(Clone)]
pub struct Providers {
    http: Client,
    pub live: bool,
    stripe_token: Option<String>,
    mercury_token: Option<String>,
    pub connected_account: Option<String>,
    destination: Option<String>,
}
impl Providers {
    pub fn configured(merchant: &str) -> Result<Self> {
        use crate::services::mercury::auth_mercury::auth_mercury_schema as mercury;
        use crate::services::stripe::auth_stripe::auth_stripe_schema as stripe;
        let stripes: Vec<stripe::Connection> =
            serde_json::from_str(&std::env::var("STRIPE_CONNECTIONS_JSON").unwrap_or("[]".into()))?;
        let mercuries: Vec<mercury::Connection> = serde_json::from_str(
            &std::env::var("MERCURY_CONNECTIONS_JSON").unwrap_or("[]".into()),
        )?;
        stripe::validate_connections(&stripes)?;
        mercury::validate_connections(&mercuries)?;
        let stripe = stripes.into_iter().find(|c| c.tenant_id == merchant);
        let mercury = mercuries.into_iter().find(|c| c.tenant_id == merchant);
        let live = std::env::var("BILLING_LIVE").as_deref() == Ok("true");
        if let Some(c) = &stripe {
            ensure!(
                c.write && matches!(c.environment, stripe::Environment::Live) == live,
                "Billing Stripe permission or environment mismatch"
            );
        }
        if let Some(c) = &mercury {
            ensure!(
                c.write && matches!(c.environment, mercury::Environment::Production) == live,
                "Billing Mercury permission or environment mismatch"
            );
        }
        ensure!(
            stripe.is_some() || mercury.is_some(),
            "No provider connection for billing merchant"
        );
        let connected_account = stripe.as_ref().and_then(|c| c.connected_account.clone());
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            http: Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(std::time::Duration::from_secs(25))
                .build()?,
            live,
            connected_account,
            stripe_token: stripe.map(|c| c.token),
            mercury_token: mercury.map(|c| c.token),
            destination: std::env::var("BILLING_MERCURY_DESTINATION").ok(),
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
        let token = self
            .stripe_token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Stripe not configured for merchant"))?;
        let mut request = self
            .http
            .request(method.clone(), format!("https://api.stripe.com/v1{path}"))
            .bearer_auth(token)
            .header("Stripe-Version", "2026-03-25.dahlia");
        if let Some(account) = &self.connected_account {
            request = request.header("Stripe-Account", account);
        }
        if method == Method::GET {
            request = request.query(form);
        } else {
            ensure!(!key.is_empty(), "Mutation requires idempotency key");
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
    pub fn ready(&self, provider: Provider) -> Result<()> {
        match provider {
            Provider::Stripe => ensure!(
                self.stripe_token.is_some(),
                "Stripe unavailable for merchant"
            ),
            Provider::Mercury => {
                ensure!(
                    self.mercury_token.is_some(),
                    "Mercury unavailable for merchant"
                );
                uuid::Uuid::parse_str(self.destination.as_deref().unwrap_or(""))?;
            }
        }
        Ok(())
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
        let key = self
            .mercury_token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Mercury not configured for merchant"))?;
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
    pub(super) async fn list_history(
        &self,
        path: &str,
        filter: &str,
        id: &str,
    ) -> Result<Vec<Value>> {
        safe_id(id)?;
        let mut values = Vec::new();
        let mut after = String::new();
        for _ in 0..10 {
            let mut params = vec![(filter.into(), id.into()), ("limit".into(), "100".into())];
            if !after.is_empty() {
                params.push(("starting_after".into(), after.clone()));
            }
            let result = self.stripe(Method::GET, path, "", &params).await?;
            let data = result["data"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Invalid history response"))?;
            values.extend(data.clone());
            if result["has_more"] == false {
                return Ok(values);
            }
            let next = field(
                data.last()
                    .ok_or_else(|| anyhow::anyhow!("Incomplete history page"))?,
                "id",
            )?;
            ensure!(next != after, "History cursor stalled");
            after = next.to_owned();
        }
        anyhow::bail!("History too large; provider review required")
    }
    pub async fn invoice(&self, input: &InvoiceRequest) -> Result<Value> {
        match input.provider {
            Provider::Mercury => {
                ensure!(
                    input.mode == Mode::Manual,
                    "Mercury API does not implement saved-card autopay"
                );
                let destination = self
                    .destination
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Missing Mercury destination"))?;
                uuid::Uuid::parse_str(destination)?;
                // Serialize a decimal JSON number from integer cents; do not calculate money in f64.
                let amount: serde_json::Number =
                    format!("{}.{:02}", input.amount / 100, input.amount % 100).parse()?;
                self.mercury(Method::POST,"/ar/invoices",Some(json!({"customerId":input.customer,"invoiceNumber":format!("JIT-{}",input.id),"destinationAccountId":destination,"currencyCode":"USD","invoiceDate":date(input.issued)?,"dueDate":date(input.due)?,"ccEmails":[],"achDebitEnabled":false,"creditCardEnabled":true,"useRealAccountNumber":false,"sendEmailOption":"DontSend","lineItems":[{"name":input.description,"quantity":1,"unitPrice":amount}]}))).await
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
                let mut invoice = self
                    .stripe(Method::GET, &format!("/invoices/{id}"), "", &[])
                    .await?;
                if invoice["status"] == "open" {
                    for payment in self
                        .list_history("/invoice_payments", "invoice", id)
                        .await?
                    {
                        ensure!(
                            payment["invoice"] == id && payment["livemode"] == self.live,
                            "Invoice payment binding mismatch"
                        );
                        if payment["status"] == "open"
                            && payment["payment"]["type"] == "payment_intent"
                        {
                            let intent_id = field(&payment["payment"], "payment_intent")?;
                            safe_id(intent_id)?;
                            let intent = self
                                .stripe(
                                    Method::GET,
                                    &format!("/payment_intents/{intent_id}"),
                                    "",
                                    &[],
                                )
                                .await?;
                            ensure!(
                                intent["customer"] == invoice["customer"]
                                    && intent["livemode"] == self.live,
                                "Payment customer mismatch"
                            );
                            if intent["status"] == "processing" {
                                invoice["business_payment_processing"] = json!(true);
                            }
                        }
                    }
                }
                Ok(invoice)
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
        (Provider::Stripe, "open") if value["business_payment_processing"] == true => "processing",
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
#[path = "../../tests/billing/providers.rs"]
mod tests;
