use super::auth_stripe_schema::{self, Connection, StripeAccess};
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogError, DogService, ServiceCapabilities, TenantContext};
use serde_json::{json, Value};
use std::sync::Arc;
pub struct AuthStripeState {
    connections: Vec<Connection>,
}
impl AuthStripeState {
    pub fn new(connections: Vec<Connection>) -> Result<Self> {
        auth_stripe_schema::validate_connections(&connections)?;
        Ok(Self { connections })
    }
    pub fn from_env() -> Result<Option<Self>> {
        match std::env::var("STRIPE_CONNECTIONS_JSON") {
            Ok(value) => {
                let connections: Vec<Connection> = serde_json::from_str(&value).map_err(|_| {
                    anyhow::anyhow!("Invalid STRIPE_CONNECTIONS_JSON configuration")
                })?;
                Ok(Some(Self::new(connections)?))
            }
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => anyhow::bail!("Invalid Stripe configuration encoding"),
        }
    }
    fn authorize(&self, tenant: &str, actor: &str, mutation: bool) -> Result<Arc<StripeAccess>> {
        let connection = self
            .connections
            .iter()
            .find(|c| c.tenant_id == tenant && c.identity_ids.contains(actor))
            .ok_or_else(|| {
                DogError::forbidden("No authorized Stripe connection for this tenant and identity")
                    .into_anyhow()
            })?;
        if mutation && !connection.write {
            return Err(DogError::forbidden("Stripe connection is read-only").into_anyhow());
        }
        Ok(Arc::new(StripeAccess {
            tenant: tenant.into(),
            actor: actor.into(),
            token: connection.token.clone(),
            environment: connection.environment,
            write: connection.write,
            connected_account: connection.connected_account.clone(),
        }))
    }
}

pub struct AuthStripeService {
    state: Arc<AuthStripeState>,
}
impl AuthStripeService {
    pub fn new(state: Arc<AuthStripeState>) -> Self {
        Self { state }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for AuthStripeService {
    fn capabilities(&self) -> ServiceCapabilities {
        super::auth_stripe_shared::capabilities()
    }
    async fn custom(
        &self,
        ctx: &TenantContext,
        method: &str,
        data: Option<Value>,
        params: BusinessParams,
    ) -> Result<Value> {
        if method != "resolve" {
            return Err(dog_core::DogError::method_not_allowed(
                "Unsupported Stripe authentication method",
            )
            .into_anyhow());
        }
        let actor = auth_stripe_schema::verified_actor(ctx, &params)?;
        let mutation = auth_stripe_schema::mutation(data.as_ref())?;
        let access = self.state.authorize(&ctx.tenant_id.0, actor, mutation)?;
        Ok(
            json!({"tenant_id":access.tenant,"identity_id":access.actor,"accessToken":access.token,
            "environment":match access.environment {super::auth_stripe_schema::Environment::Test=>"test",super::auth_stripe_schema::Environment::Live=>"live"},
            "write":access.write,"connected_account":access.connected_account}),
        )
    }
}
