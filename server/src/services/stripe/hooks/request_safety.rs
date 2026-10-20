//! Safety requirements common to all Stripe resource proxies.
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogBeforeHook, DogError, HookContext, ServiceMethodKind};
use serde_json::Value;
pub struct RequestSafety;
pub(crate) fn idempotency_key(params: &BusinessParams) -> Result<&str> {
    params
        .headers
        .get("idempotency-key")
        .map(String::as_str)
        .filter(|v| {
            !v.trim().is_empty() && v.len() <= 255 && v.bytes().all(|b| (32..=126).contains(&b))
        })
        .ok_or_else(|| {
            DogError::bad_request(
                "Stripe POST operations require a stable Idempotency-Key (1–255 ASCII characters)",
            )
            .into_anyhow()
        })
}
fn no_raw_payment_data(value: &Value) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.as_str(), "card" | "bank_account" | "us_bank_account") {
                    if let Some(fields) = value.as_object() {
                        if fields.keys().any(|k| {
                            matches!(
                                k.as_str(),
                                "number" | "cvc" | "account_number" | "routing_number"
                            )
                        }) {
                            return Err(DogError::bad_request("Collect payment details with Stripe-hosted UI or Stripe.js, not this server").into_anyhow());
                        }
                    }
                }
                no_raw_payment_data(value)?;
            }
        }
        Value::Array(rows) => {
            for row in rows {
                no_raw_payment_data(row)?;
            }
        }
        _ => {}
    }
    Ok(())
}
#[async_trait]
impl DogBeforeHook<Value, BusinessParams> for RequestSafety {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        let post = matches!(
            ctx.method,
            ServiceMethodKind::Create | ServiceMethodKind::Patch | ServiceMethodKind::Update
        ) || matches!(ctx.method,ServiceMethodKind::Custom(name) if !matches!(name,"line_items"|"payment_methods"));
        if post {
            idempotency_key(&ctx.params)?;
        }
        if let Some(data) = &ctx.data {
            no_raw_payment_data(data)?;
        }
        Ok(())
    }
}
