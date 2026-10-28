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
pub fn body(value: &Value, update: bool) -> Result<()> {
    let map = value
        .as_object()
        .ok_or_else(|| DogError::bad_request("Send one Mercury resource object").into_anyhow())?;
    require(
        value.to_string().len() <= 65536,
        "Mercury request is too large",
    )?;

    let allowed = ["name", "email", "address", "resendOpenInvoices"];
    require(
        map.keys().all(|k| allowed.contains(&k.as_str())),
        "Unknown customer field",
    )?;
    require(
        text(&value["name"]) && email(&value["email"]),
        "Customer name and email are required",
    )?;
    if update {
        require(
            value["resendOpenInvoices"].is_boolean() && map.contains_key("address"),
            "Customer update requires resendOpenInvoices and explicit address (or null)",
        )?;
    } else {
        require(
            !map.contains_key("resendOpenInvoices"),
            "resendOpenInvoices is only for updates",
        )?;
    }
    if let Some(address) = map.get("address") {
        require(
            address.is_null() || address.is_object(),
            "Invalid customer address",
        )?;
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
        Some(HookResult::One(value))
            if ctx.method == ServiceMethodKind::Remove && value.is_null() => {}
        Some(HookResult::One(value)) => resource(value)?,
        _ => return Err(DogError::unavailable("Missing Mercury response").into_anyhow()),
    }
    Ok(())
}
