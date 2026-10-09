//! Read-only provider history. Refund/dispute amounts belong to payments and are
//! not subtracted from invoice balances: Stripe permits shared payment allocations.
use super::{
    billing_schema::Provider,
    providers::{field, safe_id, Providers},
};
use anyhow::{ensure, Result};
use reqwest::Method;
use serde_json::{json, Value};
impl Providers {
    async fn list_history(&self, path: &str, filter: &str, id: &str) -> Result<Vec<Value>> {
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
    pub async fn payment_history(&self, provider: Provider, invoice: &Value) -> Result<Value> {
        if provider != Provider::Stripe {
            return Ok(
                json!({"supported":false,"message":"Detailed Mercury refund and dispute history is not exposed by this integration. Review it with Mercury.","observed_at":chrono::Utc::now().timestamp()}),
            );
        }
        let id = field(invoice, "id")?;
        let customer = field(invoice, "customer")?;
        let payments = self
            .list_history("/invoice_payments", "invoice", id)
            .await?;
        let mut history = Vec::new();
        for payment in payments {
            ensure!(
                payment["invoice"] == id
                    && payment["currency"] == "usd"
                    && payment["livemode"] == self.live,
                "Payment binding mismatch"
            );
            let mut item = json!({"id":payment["id"],"status":payment["status"],"amount_paid_cents":payment["amount_paid"],"amount_requested_cents":payment["amount_requested"],"created_at":payment["created"],"type":payment["payment"]["type"],"refunds":[],"disputes":[]});
            let charge_id = match payment["payment"]["type"].as_str() {
                Some("payment_intent") => {
                    let pi = field(&payment["payment"], "payment_intent")?;
                    safe_id(pi)?;
                    let intent = self
                        .stripe(Method::GET, &format!("/payment_intents/{pi}"), "", &[])
                        .await?;
                    ensure!(
                        intent["customer"] == customer && intent["currency"] == "usd",
                        "Payment customer mismatch"
                    );
                    intent["latest_charge"].as_str().map(str::to_owned)
                }
                Some("charge") => payment["payment"]["charge"].as_str().map(str::to_owned),
                _ => {
                    item["detail_note"] =
                        json!("Payment recorded by the provider; card-level details unavailable");
                    None
                }
            };
            if let Some(charge_id) = charge_id {
                safe_id(&charge_id)?;
                let charge = self
                    .stripe(Method::GET, &format!("/charges/{charge_id}"), "", &[])
                    .await?;
                ensure!(
                    charge["customer"] == customer && charge["currency"] == "usd",
                    "Charge customer mismatch"
                );
                item["charge_amount_cents"] = charge["amount"].clone();
                item["charge_refunded_cents"] = charge["amount_refunded"].clone();
                item["disputed"] = charge["disputed"].clone();
                item["detail_note"]=json!("Refunds and disputes below apply to this payment. A payment may cover more than one invoice; these amounts do not change the invoice balance shown above.");
                let refunds = self.list_history("/refunds", "charge", &charge_id).await?;
                let disputes = self.list_history("/disputes", "charge", &charge_id).await?;
                item["refunds"] = json!(project_adjustments(&refunds, &charge_id)?);
                item["disputes"] = json!(project_adjustments(&disputes, &charge_id)?);
            }
            history.push(item);
        }
        Ok(
            json!({"supported":true,"invoice_status":invoice["status"],"amount_paid_cents":invoice["amount_paid"],"amount_remaining_cents":invoice["amount_remaining"],"total_cents":invoice["total"],"payments":history,"observed_at":chrono::Utc::now().timestamp()}),
        )
    }
}
fn project_adjustments(values: &[Value], charge: &str) -> Result<Vec<Value>> {
    values.iter().map(|v| {ensure!(v["charge"]==charge&&v["currency"]=="usd","Adjustment binding mismatch");ensure!(v["amount"].as_i64().is_some_and(|n|n>=0),"Invalid adjustment amount");Ok(json!({"id":v["id"],"amount_cents":v["amount"],"status":v["status"],"created_at":v["created"],"reason":v["reason"]}))}).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adjustments_are_bound_and_preserve_pending_status() {
        let row = json!({"id":"re_test","charge":"ch_test","currency":"usd","amount":125,"status":"pending","created":1,"reason":"requested_by_customer","metadata":{"private":"hidden"}});
        let values = project_adjustments(std::slice::from_ref(&row), "ch_test").unwrap();
        assert_eq!(values[0]["status"], "pending");
        assert!(values[0].get("metadata").is_none());
        assert!(project_adjustments(&[row], "ch_other").is_err());
    }
}

#[cfg(test)]
mod sandbox_tests {
    use super::*;
    use crate::services::billing::{billing_schema::Mode, providers::InvoiceRequest};
    #[tokio::test]
    #[ignore = "Requires approved Stripe sandbox key; creates only synthetic payments"]
    async fn stripe_sandbox_partial_refund_dispute_history() {
        dotenvy::from_path(std::env::var("BILLING_TEST_ENV").unwrap()).unwrap();
        let p = Providers::new().unwrap();
        assert!(!p.live);
        let id = uuid::Uuid::new_v4().to_string();
        let customer = p
            .customer(
                Provider::Stripe,
                &id,
                "Sandbox history test",
                "billing-validation@example.com",
            )
            .await
            .unwrap();
        let input = InvoiceRequest {
            id: id.clone(),
            customer: customer.clone(),
            provider: Provider::Stripe,
            mode: Mode::Manual,
            amount: 1000,
            description: "Sandbox history; no real charge".into(),
            issued: chrono::Utc::now().timestamp(),
            due: chrono::Utc::now().timestamp() + 86400,
        };
        let draft = p.invoice(&input).await.unwrap();
        let invoice = field(&draft, "id").unwrap();
        p.finish_invoice(&input, invoice).await.unwrap();
        for (index, method) in ["pm_card_visa", "pm_card_createDispute"]
            .into_iter()
            .enumerate()
        {
            let params = [
                ("customer", customer.as_str()),
                ("amount", "500"),
                ("currency", "usd"),
                ("payment_method", method),
                ("payment_method_types[0]", "card"),
                ("confirm", "true"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned()));
            let pi = p
                .stripe(
                    Method::POST,
                    "/payment_intents",
                    &format!("history-{id}-{index}"),
                    &params,
                )
                .await
                .unwrap();
            assert_eq!(pi["status"], "succeeded");
            let intent = field(&pi, "id").unwrap();
            p.stripe(
                Method::POST,
                &format!("/invoices/{invoice}/attach_payment"),
                &format!("attach-{id}-{index}"),
                &[("payment_intent".into(), intent.into())],
            )
            .await
            .unwrap();
            let observed = p.observe(Provider::Stripe, invoice).await.unwrap();
            let history = p
                .payment_history(Provider::Stripe, &observed)
                .await
                .unwrap();
            assert_eq!(history["amount_paid_cents"], 500 * (index + 1));
            assert_eq!(history["amount_remaining_cents"], 500 * (1 - index));
            if index == 0 {
                p.stripe(
                    Method::POST,
                    "/refunds",
                    &format!("refund-{id}"),
                    &[
                        ("payment_intent".into(), intent.into()),
                        ("amount".into(), "100".into()),
                    ],
                )
                .await
                .unwrap();
                let observed = p.observe(Provider::Stripe, invoice).await.unwrap();
                let history = p
                    .payment_history(Provider::Stripe, &observed)
                    .await
                    .unwrap();
                assert_eq!(history["amount_remaining_cents"], 500);
                assert!(history["payments"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["refunds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r["amount_cents"] == 100 && r["status"] == "succeeded")));
            }
        }
        let mut found = false;
        for _ in 0..12 {
            let observed = p.observe(Provider::Stripe, invoice).await.unwrap();
            let history = p
                .payment_history(Provider::Stripe, &observed)
                .await
                .unwrap();
            if history["payments"].as_array().unwrap().iter().any(|v| {
                v["disputes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["amount_cents"] == 500)
            }) {
                found = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        assert!(found, "Sandbox dispute not observed");
        println!(
            "HISTORY verified partial payment, refund, full payment and dispute; sandbox only"
        );
    }
}
