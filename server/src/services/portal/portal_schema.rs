use crate::services::BusinessParams;
use dog_core::{DogError, TenantContext};
use serde_json::Value;
pub fn actor(ctx: &TenantContext, p: &BusinessParams) -> anyhow::Result<String> {
    let u = p
        .auth_result
        .as_ref()
        .and_then(|v| v.get("user"))
        .ok_or_else(|| DogError::not_authenticated("Please sign in").into_anyhow())?;
    if !p.authenticated || u["tenant_id"].as_str() != Some(&ctx.tenant_id.0) {
        return Err(DogError::not_authenticated("Company access changed").into_anyhow());
    }
    crate::billing::billing_schema::id(u["id"].as_str().unwrap_or(""))
}
pub fn field<'a>(v: &'a Value, k: &str) -> anyhow::Result<&'a str> {
    v[k].as_str()
        .filter(|s| !s.is_empty() && s.len() < 512)
        .ok_or_else(|| DogError::bad_request("Required information is missing").into_anyhow())
}
