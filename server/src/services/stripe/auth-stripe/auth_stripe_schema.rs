//! Server-only API-token connection and credential contracts.
use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::{DogError, TenantContext};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Test,
    Live,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub tenant_id: String,
    pub identity_ids: HashSet<String>,
    pub environment: Environment,
    pub token: String,
    #[serde(default)]
    pub connected_account: Option<String>,
    #[serde(default)]
    pub write: bool,
}
/// Issued by the authorization hook; cannot be constructed from request JSON.
pub struct StripeAccess {
    pub(crate) tenant: String,
    pub(crate) actor: String,
    pub(crate) token: String,
    pub(crate) environment: Environment,
    pub(crate) write: bool,
    pub(crate) connected_account: Option<String>,
}
impl std::fmt::Debug for StripeAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StripeAccess")
            .field("tenant", &self.tenant)
            .field("environment", &self.environment)
            .finish_non_exhaustive()
    }
}

impl StripeAccess {
    pub(crate) fn for_request<'a>(
        ctx: &TenantContext,
        params: &'a BusinessParams,
        mutation: bool,
    ) -> Result<&'a Self> {
        let access = params.inner.stripe_access.as_deref().ok_or_else(|| {
            DogError::not_authenticated("Stripe credentials have not been resolved").into_anyhow()
        })?;
        let actor = verified_actor(ctx, params)?;
        if access.tenant != ctx.tenant_id.0 || access.actor != actor || (mutation && !access.write)
        {
            return Err(
                DogError::forbidden("Stripe credentials do not match request").into_anyhow(),
            );
        }
        Ok(access)
    }
    pub(crate) fn from_result(
        value: Value,
        ctx: &TenantContext,
        params: &BusinessParams,
    ) -> Result<Arc<Self>> {
        let actor = verified_actor(ctx, params)?;
        if value["tenant_id"].as_str() != Some(ctx.tenant_id.0.as_str())
            || value["identity_id"].as_str() != Some(actor)
        {
            return Err(
                DogError::forbidden("Resolved Stripe connection does not match identity")
                    .into_anyhow(),
            );
        }
        let token = value["accessToken"]
            .as_str()
            .filter(|v| !v.trim().is_empty() && !v.chars().any(char::is_control))
            .ok_or_else(|| {
                DogError::unavailable("Invalid resolved Stripe credentials").into_anyhow()
            })?;
        let environment: Environment = serde_json::from_value(value["environment"].clone())
            .map_err(|_| DogError::unavailable("Invalid Stripe environment").into_anyhow())?;
        validate_key(token, environment)?;
        validate_account(value["connected_account"].as_str())?;
        let write = value["write"]
            .as_bool()
            .ok_or_else(|| DogError::unavailable("Invalid Stripe permission").into_anyhow())?;
        Ok(Arc::new(Self {
            tenant: ctx.tenant_id.0.clone(),
            actor: actor.into(),
            token: token.into(),
            environment,
            write,
            connected_account: value["connected_account"].as_str().map(str::to_owned),
        }))
    }
}
pub(crate) fn verified_actor<'a>(
    ctx: &TenantContext,
    params: &'a BusinessParams,
) -> Result<&'a str> {
    let user = params
        .auth_result
        .as_ref()
        .and_then(|v| v.get("user"))
        .filter(|_| params.authenticated)
        .ok_or_else(|| {
            DogError::not_authenticated("Verified Business identity required").into_anyhow()
        })?;
    if user["tenant_id"].as_str() != Some(ctx.tenant_id.0.as_str()) {
        return Err(DogError::forbidden("Verified tenant does not match request").into_anyhow());
    }
    user["id"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| DogError::not_authenticated("Missing verified identity").into_anyhow())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveRequest {
    operation: Operation,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Operation {
    Read,
    Write,
}
pub(crate) fn validate_connections(connections: &[Connection]) -> Result<()> {
    let mut tenants = HashSet::new();
    for c in connections {
        validate_key(&c.token, c.environment)?;
        validate_account(c.connected_account.as_deref())?;
        anyhow::ensure!(
            !c.tenant_id.trim().is_empty() && tenants.insert(c.tenant_id.clone()),
            "Stripe requires one explicit connection per tenant"
        );
        anyhow::ensure!(
            !c.identity_ids.is_empty() && c.identity_ids.iter().all(|id| !id.trim().is_empty()),
            "Stripe requires authorized identities"
        );
        anyhow::ensure!(
            !c.token.trim().is_empty() && !c.token.chars().any(char::is_control),
            "Invalid Stripe token"
        );
    }
    Ok(())
}
pub(crate) fn mutation(data: Option<&Value>) -> Result<bool> {
    let request: ResolveRequest = serde_json::from_value(data.cloned().unwrap_or(Value::Null))
        .map_err(|_| DogError::bad_request("Send only operation: read or write").into_anyhow())?;
    Ok(matches!(request.operation, Operation::Write))
}

pub(crate) fn validate_key(token: &str, environment: Environment) -> Result<()> {
    let valid = match environment {
        Environment::Test => token.starts_with("sk_test_") || token.starts_with("rk_test_"),
        Environment::Live => token.starts_with("sk_live_") || token.starts_with("rk_live_"),
    };
    anyhow::ensure!(
        valid && token.len() > 12 && !token.chars().any(|c| c.is_control() || c.is_whitespace()),
        "Stripe key does not match configured environment"
    );
    Ok(())
}
fn validate_account(account: Option<&str>) -> Result<()> {
    anyhow::ensure!(
        account.is_none_or(|a| a.starts_with("acct_")
            && a.len() > 5
            && a.len() <= 255
            && a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')),
        "Invalid configured Stripe account"
    );
    Ok(())
}
impl StripeAccess {
    pub(crate) fn live(&self) -> bool {
        matches!(self.environment, Environment::Live)
    }
}
