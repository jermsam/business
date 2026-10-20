use super::auth_stripe_schema::{self, StripeAccess};
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{hooks::HookResult, DogAfterHook, DogBeforeHook, DogError, HookContext};
use serde_json::Value;

pub struct BeforeResolve;
#[async_trait]
impl DogBeforeHook<Value, BusinessParams> for BeforeResolve {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        ctx.params.inner.stripe_access = None;
        if ctx
            .params
            .provider
            .as_deref()
            .is_some_and(|p| !p.is_empty())
            || !ctx.params.inner.provider.is_empty()
        {
            return Err(DogError::forbidden("Stripe credentials are server-only").into_anyhow());
        }
        auth_stripe_schema::verified_actor(&ctx.tenant, &ctx.params)?;
        auth_stripe_schema::mutation(ctx.data.as_ref())?;
        Ok(())
    }
}
pub struct AfterResolve;
#[async_trait]
impl DogAfterHook<Value, BusinessParams> for AfterResolve {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        let Some(HookResult::One(value)) = &ctx.result else {
            return Err(DogError::unavailable("Missing Stripe credential result").into_anyhow());
        };
        StripeAccess::from_result(value.clone(), &ctx.tenant, &ctx.params)?;
        Ok(())
    }
}
