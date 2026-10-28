use crate::services::BusinessParams;
use dog_core::{DogBeforeHook, HookContext};
use serde_json::Value;
pub struct Verify;
#[async_trait::async_trait]
impl DogBeforeHook<Value, BusinessParams> for Verify {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> anyhow::Result<()> {
        super::portal_schema::actor(&ctx.tenant, &ctx.params)?;
        Ok(())
    }
}
