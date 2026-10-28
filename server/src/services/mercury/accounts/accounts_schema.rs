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
pub fn validate(ctx: &HookContext<Value, BusinessParams>) -> Result<()> {
    if ctx.method == ServiceMethodKind::Find {
        query(&ctx.params.inner.query)?;
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

        Some(HookResult::One(value)) => resource(value)?,
        _ => return Err(DogError::unavailable("Missing Mercury response").into_anyhow()),
    }
    Ok(())
}
