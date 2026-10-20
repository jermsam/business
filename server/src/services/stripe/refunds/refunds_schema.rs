use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::{DogError, HookContext, ServiceMethodKind};
use serde_json::Value;
fn ensure(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(DogError::bad_request(message).into_anyhow())
    }
}
fn id(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
        && s.len() > prefix.len()
        && s.len() <= 255
        && !s.contains("_secret_")
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn currency(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| s.len() == 3 && s.bytes().all(|b| b.is_ascii_lowercase()))
}
pub fn validate(ctx: &HookContext<Value, BusinessParams>) -> Result<()> {
    if ctx.method == ServiceMethodKind::Find {
        ensure(
            !(ctx.params.inner.query.contains_key("starting_after")
                && ctx.params.inner.query.contains_key("ending_before")),
            "Use one pagination direction",
        )?;
        for (k, v) in &ctx.params.inner.query {
            ensure(
                [
                    "limit",
                    "starting_after",
                    "ending_before",
                    "created",
                    "created[gt]",
                    "created[gte]",
                    "created[lt]",
                    "created[lte]",
                    "charge",
                    "payment_intent",
                ]
                .contains(&k.as_str()),
                "Unsupported query parameter",
            )?;
            if k == "limit" {
                ensure(
                    v.parse::<u16>().is_ok_and(|n| (1..=100).contains(&n)),
                    "limit must be 1–100",
                )?;
            }
            if k == "starting_after" || k == "ending_before" {
                ensure(id(v, "re_"), "Invalid pagination cursor")?;
            }
            if k.starts_with("created") {
                ensure(v.parse::<u64>().is_ok(), "Invalid timestamp")?;
            }
            ensure(
                v.len() <= 1000 && !v.chars().any(char::is_control),
                "Invalid query value",
            )?;
        }
        return Ok(());
    }
    ensure(
        ctx.params.inner.query.is_empty(),
        "Query parameters unsupported for this operation",
    )?;
    if matches!(
        ctx.method,
        ServiceMethodKind::Get | ServiceMethodKind::Remove
    ) {
        return Ok(());
    }
    let data = ctx
        .data
        .as_ref()
        .ok_or_else(|| DogError::bad_request("Request object required").into_anyhow())?;
    let map = data
        .as_object()
        .ok_or_else(|| DogError::bad_request("Request object required").into_anyhow())?;
    ensure(data.to_string().len() <= 65536, "Request too large")?;
    let mut allowed: &[&str] = &["amount", "charge", "payment_intent", "reason", "metadata"];
    match ctx.method {
        ServiceMethodKind::Patch => allowed = &["metadata"],
        ServiceMethodKind::Custom(method) => match method {
            "cancel" => {
                allowed = &["id"];
            }
            _ => return Err(DogError::method_not_allowed("Unsupported method").into_anyhow()),
        },
        _ => {}
    }
    ensure(
        map.keys().all(|k| allowed.contains(&k.as_str())),
        "Unsupported resource field",
    )?;
    if matches!(ctx.method, ServiceMethodKind::Custom(_)) {
        ensure(
            data["id"].as_str().is_some_and(|s| id(s, "re_")),
            "Resource id required",
        )?;
    }
    for (k, v) in map {
        if matches!(k.as_str(), "email" | "customer_email" | "receipt_email") {
            ensure(
                v.as_str().is_some_and(|s| {
                    s.is_empty()
                        || (s.len() <= 254
                            && s.split_once('@').is_some_and(|(a, b)| {
                                !a.is_empty()
                                    && b.contains('.')
                                    && !s.chars().any(char::is_whitespace)
                            }))
                }),
                "Invalid email",
            )?;
        }
        if matches!(
            k.as_str(),
            "auto_advance"
                | "off_session"
                | "confirm"
                | "submit"
                | "final_capture"
                | "forgive"
                | "paid_out_of_band"
                | "discountable"
                | "allow_promotion_codes"
        ) {
            ensure(v.is_boolean(), "Boolean field required")?;
        }
        if matches!(k.as_str(), "quantity" | "amount_to_capture") {
            ensure(
                v.as_u64().is_some_and(|n| n > 0),
                "Positive integer required",
            )?;
        }
        if k == "amount" {
            ensure(
                v.as_u64().is_some_and(|n| n > 0),
                "Positive integer amount required",
            )?;
        }
        if k == "limit" {
            ensure(
                v.as_u64().is_some_and(|n| (1..=100).contains(&n)),
                "limit must be 1–100",
            )?;
        }
        if k == "starting_after" || k == "ending_before" {
            ensure(
                v.as_str().is_some_and(|s| id(s, "re_")),
                "Invalid pagination cursor",
            )?;
        }

        if let Some(prefix) = match k.as_str() {
            "customer" => Some("cus_"),
            "payment_intent" => Some("pi_"),
            "payment_method" | "default_payment_method" => Some("pm_"),
            "invoice" => Some("in_"),
            "charge" => Some("ch_"),
            "mandate" => Some("mandate_"),
            _ => None,
        } {
            ensure(
                v.as_str().is_some_and(|s| id(s, prefix)),
                "Invalid referenced resource",
            )?;
        }
        if matches!(k.as_str(), "amount" | "amount_to_capture" | "quantity") {
            ensure(
                v.as_i64().is_some(),
                "Amounts and quantities must be integers",
            )?;
        }
        if k == "currency" {
            ensure(currency(v), "Invalid currency")?;
        }
        if matches!(k.as_str(), "success_url" | "cancel_url" | "return_url") {
            ensure(
                v.as_str()
                    .and_then(|s| reqwest::Url::parse(s).ok())
                    .is_some_and(|u| {
                        u.scheme() == "https"
                            && u.username().is_empty()
                            && u.password().is_none()
                            && u.host_str().is_some()
                    }),
                "HTTPS return URL required",
            )?;
        }
        if k == "metadata" {
            ensure(
                v.is_null()
                    || v.as_str() == Some("")
                    || v.as_object().is_some_and(|m| {
                        m.len() <= 50
                            && m.iter().all(|(k, v)| {
                                k.len() <= 40 && v.as_str().is_some_and(|s| s.len() <= 500)
                            })
                    }),
                "Invalid metadata",
            )?;
        }
    }
    if ctx.method == ServiceMethodKind::Create {
        ensure(
            data.get("charge").is_some() != data.get("payment_intent").is_some(),
            "Specify either charge or payment_intent",
        )?;
    }
    Ok(())
}
pub fn response(ctx: &HookContext<Value, BusinessParams>) -> Result<()> {
    use dog_core::hooks::HookResult;
    fn resource(v: &Value, object: &str, prefix: &str) -> Result<()> {
        ensure(
            v["object"] == object && v["id"].as_str().is_some_and(|s| id(s, prefix)),
            "Invalid Stripe resource response",
        )
    }
    match &ctx.result {
        Some(HookResult::Many(rows)) => {
            for row in rows {
                resource(row, "refund", "re_")?;
            }
        }
        Some(HookResult::One(v)) => {
            resource(v, "refund", "re_")?;
            if ctx.method == ServiceMethodKind::Remove {
                ensure(v["deleted"] == true, "Deletion not confirmed")?;
            }
        }
        _ => return Err(DogError::unavailable("Missing Stripe response").into_anyhow()),
    }
    Ok(())
}
