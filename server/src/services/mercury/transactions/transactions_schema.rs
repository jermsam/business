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
    let mut out = Vec::new();
    require(
        !values.contains_key("start_after") || !values.contains_key("end_before"),
        "Use one pagination direction",
    )?;
    require(
        !values.contains_key("start_at")
            || (!values.contains_key("start_after") && !values.contains_key("end_before")),
        "Use one pagination cursor",
    )?;
    for (key, value) in values {
        require(
            value.len() <= 2048 && !value.chars().any(char::is_control),
            "Invalid Mercury query value",
        )?;
        match key.as_str() {
            "limit" => require(
                value.parse::<u16>().is_ok_and(|v| (1..=1000).contains(&v)),
                "limit must be 1–1000",
            )?,
            "order" => require(
                matches!(value.as_str(), "asc" | "desc"),
                "order must be asc or desc",
            )?,
            "start_after" | "end_before" => id(value)?,
            "start_at" => id(value)?,
            "accountId" | "status" | "cardId" => {
                for item in value.split(',') {
                    require(!item.is_empty(), "Empty transaction filter")?;
                    if key == "accountId" {
                        id(item)?;
                    }
                    if key == "status" {
                        require(
                            matches!(
                                item,
                                "pending"
                                    | "sent"
                                    | "cancelled"
                                    | "failed"
                                    | "reversed"
                                    | "blocked"
                            ),
                            "Invalid transaction status",
                        )?;
                    }
                    out.push((key.clone(), item.into()));
                }
                continue;
            }
            "start" | "end" | "postedStart" | "postedEnd" => {
                require(
                    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
                        || chrono::DateTime::parse_from_rfc3339(value).is_ok(),
                    "Invalid transaction date",
                )?;
            }
            "categoryId" => id(value)?,
            "search" | "mercuryCategory" => {}
            _ => {
                return Err(
                    DogError::bad_request("Unsupported Mercury query parameter").into_anyhow()
                )
            }
        }
        out.push((key.clone(), value.clone()));
    }
    out.sort();
    Ok(out)
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
