use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::{DogError, HookContext, ServiceMethodKind};
use serde_json::Value;
use std::collections::HashMap;
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(DogError::bad_request(message).into_anyhow())
    }
}
pub fn id(value: &str) -> Result<()> {
    require(
        uuid::Uuid::parse_str(value).is_ok(),
        "Mercury IDs must be UUIDs",
    )
}
pub fn query(values: &HashMap<String, String>) -> Result<Vec<(String, String)>> {
    require(
        !(values.contains_key("start_after") && values.contains_key("end_before")),
        "Use one pagination direction",
    )?;
    let mut query = Vec::new();
    for (key, value) in values {
        match key.as_str() {
            "limit" => require(
                value.parse::<u16>().is_ok_and(|n| (1..=1000).contains(&n)),
                "limit must be 1–1000",
            )?,
            "order" => require(
                matches!(value.as_str(), "asc" | "desc"),
                "Invalid sort order",
            )?,
            "start_after" | "end_before" => id(value)?,
            _ => {
                return Err(
                    DogError::bad_request("Unsupported resource query parameter").into_anyhow(),
                )
            }
        }
        query.push((key.clone(), value.clone()));
    }
    query.sort();
    Ok(query)
}
fn text(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        !s.trim().is_empty() && s.len() <= 10000 && !s.chars().any(char::is_control)
    })
}
fn email(v: &Value) -> bool {
    text(v)
        && v.as_str().is_some_and(|s| {
            s.len() <= 254
                && s.split_once('@')
                    .is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !s.contains(' '))
        })
}
fn date(v: &Value) -> bool {
    v.as_str()
        .is_some_and(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok())
}
fn money(v: &Value) -> bool {
    // Validate the JSON decimal as supplied; never round money or compute totals.
    if !v.is_number() {
        return false;
    }
    let s = v.to_string();
    !s.starts_with('-')
        && !s.contains(['e', 'E'])
        && s.split_once('.')
            .is_none_or(|(_, fraction)| fraction.len() <= 2)
}
pub fn body(value: &Value, update: bool) -> Result<()> {
    let map = value
        .as_object()
        .ok_or_else(|| DogError::bad_request("Send one Mercury resource object").into_anyhow())?;
    require(
        value.to_string().len() <= 65536,
        "Mercury request is too large",
    )?;

    let allowed = [
        "achDebitEnabled",
        "ccEmails",
        "creditCardEnabled",
        "currencyCode",
        "customerId",
        "destinationAccountId",
        "dueDate",
        "internalNote",
        "invoiceDate",
        "invoiceNumber",
        "lineItems",
        "payerMemo",
        "poNumber",
        "sendEmailOption",
        "servicePeriodEndDate",
        "servicePeriodStartDate",
        "useRealAccountNumber",
    ];
    require(
        map.keys().all(|k| allowed.contains(&k.as_str())),
        "Unknown invoice field",
    )?;
    for k in [
        "achDebitEnabled",
        "creditCardEnabled",
        "useRealAccountNumber",
    ] {
        require(
            value[k].is_boolean(),
            "Invoice payment flags must be explicit booleans",
        )?;
    }
    require(
        value["ccEmails"]
            .as_array()
            .is_some_and(|v| v.len() <= 100 && v.iter().all(email)),
        "ccEmails must be an email array",
    )?;
    require(
        date(&value["invoiceDate"]) && date(&value["dueDate"]),
        "Invoice dates must use YYYY-MM-DD",
    )?;
    if update {
        require(
            text(&value["invoiceNumber"]),
            "Full invoice update requires invoiceNumber",
        )?;
        require(
            !map.contains_key("customerId")
                && !map.contains_key("destinationAccountId")
                && !map.contains_key("currencyCode")
                && !map.contains_key("sendEmailOption"),
            "Invoice update cannot change creation-only fields",
        )?;
        for k in [
            "internalNote",
            "payerMemo",
            "poNumber",
            "servicePeriodEndDate",
            "servicePeriodStartDate",
        ] {
            require(
                map.contains_key(k),
                "Full invoice update must explicitly retain or clear optional fields",
            )?;
        }
    } else {
        for k in ["customerId", "destinationAccountId"] {
            id(value[k].as_str().unwrap_or(""))?;
        }
        require(
            matches!(
                value["sendEmailOption"].as_str(),
                Some("SendNow" | "DontSend")
            ),
            "Choose SendNow or DontSend explicitly",
        )?;
        if let Some(code) = map.get("currencyCode").filter(|v| !v.is_null()) {
            require(
                code.as_str()
                    .is_some_and(|s| s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase())),
                "Invalid currencyCode",
            )?;
        }
    }
    for k in ["internalNote", "payerMemo", "poNumber", "invoiceNumber"] {
        if let Some(v) = map.get(k).filter(|v| !v.is_null()) {
            require(v.is_string(), "Invoice text fields must be strings or null")?;
        }
    }
    for k in ["servicePeriodStartDate", "servicePeriodEndDate"] {
        if let Some(v) = map.get(k).filter(|v| !v.is_null()) {
            require(date(v), "Invalid service period date")?;
        }
    }
    let items = value["lineItems"]
        .as_array()
        .ok_or_else(|| DogError::bad_request("lineItems must be an array").into_anyhow())?;
    require(
        !items.is_empty() && items.len() <= 200,
        "Invoice requires 1–200 line items",
    )?;
    for item in items {
        require(
            item.as_object().is_some_and(|m| {
                m.keys().all(|k| {
                    ["name", "quantity", "unitPrice", "salesTaxRate"].contains(&k.as_str())
                })
            }),
            "Invalid line item fields",
        )?;
        require(
            text(&item["name"])
                && money(&item["unitPrice"])
                && item["quantity"]
                    .as_f64()
                    .is_some_and(|n| n.is_finite() && n > 0.0),
            "Invalid line item name, quantity or decimal unitPrice",
        )?;
        if let Some(rate) = item.get("salesTaxRate").filter(|v| !v.is_null()) {
            require(
                rate.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0),
                "Invalid salesTaxRate",
            )?;
        }
    }

    Ok(())
}
pub fn validate(ctx: &HookContext<Value, BusinessParams>) -> Result<()> {
    match ctx.method {
        ServiceMethodKind::Find => {
            query(&ctx.params.inner.query)?;
        }
        ServiceMethodKind::Create | ServiceMethodKind::Update => {
            require(
                ctx.params.inner.query.is_empty(),
                "Mutation query parameters are not supported",
            )?;
            body(
                ctx.data.as_ref().unwrap_or(&Value::Null),
                ctx.method == ServiceMethodKind::Update,
            )?;
        }
        ServiceMethodKind::Custom("cancel" | "pdf") => {
            require(
                ctx.params.inner.query.is_empty(),
                "Custom invoice query parameters are not supported",
            )?;
            let data = ctx
                .data
                .as_ref()
                .and_then(Value::as_object)
                .ok_or_else(|| DogError::bad_request("Send an invoice id").into_anyhow())?;
            require(data.len() == 1, "Send only the invoice id")?;
            id(data.get("id").and_then(Value::as_str).unwrap_or(""))?;
        }
        _ => {}
    }
    Ok(())
}
pub fn verify_response(ctx: &HookContext<Value, BusinessParams>) -> Result<()> {
    use dog_core::hooks::HookResult;
    fn resource(value: &Value) -> Result<()> {
        let id = value["id"].as_str().ok_or_else(|| {
            DogError::unavailable("Invalid Mercury resource response").into_anyhow()
        })?;
        self::id(id)
            .map_err(|_| DogError::unavailable("Invalid Mercury resource identity").into_anyhow())
    }
    match &ctx.result {
        Some(HookResult::Many(rows)) => {
            for row in rows {
                resource(row)?;
            }
        }

        Some(HookResult::One(value)) if ctx.method == ServiceMethodKind::Custom("pdf") => {
            anyhow::ensure!(
                value["contentType"] == "application/pdf"
                    && value["encoding"] == "base64"
                    && value["data"].is_string(),
                "Invalid Mercury PDF result"
            );
        }
        Some(HookResult::One(value)) => resource(value)?,
        _ => return Err(DogError::unavailable("Missing Mercury response").into_anyhow()),
    }
    Ok(())
}
