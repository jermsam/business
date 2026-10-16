//! Owner-only sandbox bridge. Product service credentials never leave the server.
use super::http::{self, Application, Input};
use crate::{
    services::{authentication::AuthService, workspace::WorkspaceService, BusinessParams},
    typedb::TypeDBState,
};
use anyhow::{ensure, Result};
use async_trait::async_trait;
use dog_core::{
    tenant::TenantContext, DogError, DogService, ServiceCapabilities, ServiceMethodKind,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;

pub struct Browser {
    access: WorkspaceService,
    merchant: String,
    router: axum::Router,
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    action: String,
    request: Option<Input>,
}
impl Browser {
    pub fn new(state: Arc<TypeDBState>, auth: Arc<AuthService>) -> Result<Self> {
        ensure!(
            state.database == "business_dev",
            "Sandbox database required"
        );
        let apps: Vec<Application> =
            serde_json::from_str(&std::env::var("PAYMENTS_APPLICATIONS_JSON")?)?;
        let id = std::env::var("PAYMENTS_BROWSER_APP")?;
        let mut app = apps
            .into_iter()
            .find(|app| app.id == id)
            .ok_or_else(|| anyhow::anyhow!("Browser sandbox application missing"))?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        app.token_sha256 = format!("{:x}", Sha256::digest(token.as_bytes()));
        Ok(Self {
            access: WorkspaceService::new(state.clone(), auth),
            merchant: app.merchant.clone(),
            router: http::router(state, vec![app])?,
            token,
        })
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for Browser {
    fn capabilities(&self) -> ServiceCapabilities {
        ServiceCapabilities::from_methods(vec![ServiceMethodKind::Create])
    }
    async fn create(
        &self,
        ctx: &TenantContext,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        let scope = self.access.scope(ctx, params).await?;
        let rows = self.access.query(format!(r#"{scope} $t has biz_id {}; let $owner=biz_owner($u,$t,$now); $owner==true; fetch {{"id":$t.biz_id}};"#, serde_json::to_string(&self.merchant)?), false).await?;
        if rows.len() != 1 {
            return Err(
                DogError::forbidden("Sandbox requires the configured merchant owner").into_anyhow(),
            );
        }
        let command: Command = serde_json::from_value(data)
            .map_err(|_| DogError::bad_request("Invalid sandbox command").into_anyhow())?;
        if command.action == "info" {
            return Ok(json!({"sandbox":true,"max_amount_minor":10000,"currency":"USD"}));
        }
        let path = match command.action.as_str() {
            "create" => "/payments/v1/checkouts",
            "status" => "/payments/v1/status",
            "reconcile" => "/payments/v1/reconcile",
            "dispatch" => "/payments/v1/events/dispatch",
            _ => return Err(DogError::bad_request("Unknown sandbox action").into_anyhow()),
        };
        let input = command
            .request
            .ok_or_else(|| DogError::bad_request("Payment request required").into_anyhow())?;
        ensure!(
            input.currency == "USD"
                && input.currency_exponent == 2
                && (1..=10000).contains(&input.amount_minor),
            "Sandbox panel accepts USD 0.01–100 only"
        );
        let body = serde_json::to_vec(&input)?;
        let response = self
            .router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {}", self.token))
                    .body(axum::body::Body::from(body))?,
            )
            .await?;
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 16384).await?;
        if !status.is_success() {
            return Err(DogError::bad_request(format!("Sandbox operation returned {}. Check status before retrying; unpaid sessions cannot reconcile as paid.", status.as_u16())).into_anyhow());
        }
        Ok(serde_json::from_slice(&body)?)
    }
}
