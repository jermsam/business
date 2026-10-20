//! Shared resource hook: obtain credentials through auth-stripe's service pipeline.
use super::super::auth_stripe::auth_stripe_schema::StripeAccess;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogBeforeHook, HookContext, ServiceMethodKind};
use serde_json::{json, Value};
/// Resource hooks call the registered auth service, including its strict hooks.
pub struct ResolveCredentials;
#[async_trait]
impl DogBeforeHook<Value, BusinessParams> for ResolveCredentials {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        ctx.params.inner.stripe_access = None;
        let write = !matches!(
            ctx.method,
            ServiceMethodKind::Find
                | ServiceMethodKind::Get
                | ServiceMethodKind::Custom("line_items" | "payment_methods")
        );
        let mut params = ctx.params.clone();
        params.provider = None;
        params.inner.provider.clear();
        params.inner.query.clear();
        let result = ctx
            .services
            .app()
            .service("auth-stripe")?
            .custom(
                ctx.tenant.clone(),
                "resolve",
                Some(json!({"operation":if write {"write"}else{"read"}})),
                params,
            )
            .await?;
        ctx.params.inner.stripe_access =
            Some(StripeAccess::from_result(result, &ctx.tenant, &ctx.params)?);
        Ok(())
    }
}
