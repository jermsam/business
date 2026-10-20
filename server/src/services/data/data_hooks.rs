use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{
    hooks::{DogAfterHook, DogBeforeHook, HookContext, HookResult},
    DogError,
};
use serde_json::Value;
use std::sync::Arc;

use super::data_schema::payload as validate_payload;

pub struct BeforeRead;
pub struct BeforeWrite;
fn bind_scope(ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
    // The preceding dog-auth strict hook owns authentication. Never reuse a scope.
    ctx.params.inner.schema_policy_scope = None;
    anyhow::ensure!(
        ctx.params.authenticated,
        "Authentication hook must run first"
    );
    let verified = ctx.params.auth_result.as_ref().ok_or_else(|| {
        DogError::not_authenticated("Missing verified authentication").into_anyhow()
    })?;
    let user = &verified["user"];
    let id = user["id"]
        .as_str()
        .ok_or_else(|| DogError::not_authenticated("Missing verified identity").into_anyhow())?;
    if user["tenant_id"].as_str() != Some(ctx.tenant.tenant_id.0.as_str()) {
        return Err(DogError::not_authenticated("Tenant membership not verified").into_anyhow());
    }
    validate_payload(ctx.data.as_ref())?;
    ctx.params.inner.schema_policy_scope = Some(Arc::new(
        dog_typedb::isolation::AuthenticatedScope::new(&ctx.tenant.tenant_id.0, id)?,
    ));
    // No query registry, rewriting or domain decisions here.
    Ok(())
}
#[async_trait]
impl DogBeforeHook<Value, BusinessParams> for BeforeRead {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        bind_scope(ctx)
    }
}
#[async_trait]
impl DogBeforeHook<Value, BusinessParams> for BeforeWrite {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        bind_scope(ctx)
    }
}
pub struct AfterRead;
pub struct AfterWrite;
fn verify_response(ctx: &HookContext<Value, BusinessParams>) -> Result<()> {
    let Some(HookResult::One(envelope)) = &ctx.result else {
        anyhow::bail!("Missing data result");
    };
    anyhow::ensure!(
        envelope.pointer("/ok/answers").is_some_and(Value::is_array)
            && envelope.pointer("/ok/query").is_none(),
        "Unsafe data response"
    );
    Ok(())
}
#[async_trait]
impl DogAfterHook<Value, BusinessParams> for AfterRead {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        verify_response(ctx)
    }
}
#[async_trait]
impl DogAfterHook<Value, BusinessParams> for AfterWrite {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        verify_response(ctx)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/services/data/data_hooks/tests.rs"]
mod tests;
