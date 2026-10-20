use super::auth_mercury_schema::{self, Connection, MercuryAccess};
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogError, DogService, ServiceCapabilities, TenantContext};
use serde_json::{json, Value};
use std::sync::Arc;
pub struct AuthMercuryState {
    connections: Vec<Connection>,
}
impl AuthMercuryState {
    pub fn new(connections: Vec<Connection>) -> Result<Self> {
        auth_mercury_schema::validate_connections(&connections)?;
        Ok(Self { connections })
    }
    pub fn from_env() -> Result<Option<Self>> {
        match std::env::var("MERCURY_CONNECTIONS_JSON") {
            Ok(value) => {
                let connections: Vec<Connection> = serde_json::from_str(&value).map_err(|_| {
                    anyhow::anyhow!("Invalid MERCURY_CONNECTIONS_JSON configuration")
                })?;
                Ok(Some(Self::new(connections)?))
            }
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => anyhow::bail!("Invalid Mercury configuration encoding"),
        }
    }
    fn authorize(&self, tenant: &str, actor: &str, mutation: bool) -> Result<Arc<MercuryAccess>> {
        let connection = self
            .connections
            .iter()
            .find(|c| c.tenant_id == tenant && c.identity_ids.contains(actor))
            .ok_or_else(|| {
                DogError::forbidden("No authorized Mercury connection for this tenant and identity")
                    .into_anyhow()
            })?;
        if mutation && !connection.write {
            return Err(DogError::forbidden("Mercury connection is read-only").into_anyhow());
        }
        Ok(Arc::new(MercuryAccess {
            tenant: tenant.into(),
            actor: actor.into(),
            token: connection.token.clone(),
            environment: connection.environment,
            write: connection.write,
        }))
    }
}

pub struct AuthMercuryService {
    state: Arc<AuthMercuryState>,
}
impl AuthMercuryService {
    pub fn new(state: Arc<AuthMercuryState>) -> Self {
        Self { state }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for AuthMercuryService {
    fn capabilities(&self) -> ServiceCapabilities {
        super::auth_mercury_shared::capabilities()
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
                "Unsupported Mercury authentication method",
            )
            .into_anyhow());
        }
        let actor = auth_mercury_schema::verified_actor(ctx, &params)?;
        let mutation = auth_mercury_schema::mutation(data.as_ref())?;
        let access = self.state.authorize(&ctx.tenant_id.0, actor, mutation)?;
        Ok(
            json!({"tenant_id":access.tenant,"identity_id":access.actor,"accessToken":access.token,
            "environment":match access.environment {super::auth_mercury_schema::Environment::Sandbox=>"sandbox",super::auth_mercury_schema::Environment::Production=>"production"},
            "write":access.write}),
        )
    }
}
