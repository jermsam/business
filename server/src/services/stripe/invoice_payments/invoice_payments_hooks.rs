use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogAfterHook, DogBeforeHook, HookContext};
use serde_json::Value;
pub struct BeforeRequest;
#[async_trait]
impl DogBeforeHook<Value, BusinessParams> for BeforeRequest {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        super::invoice_payments_schema::validate(ctx)
    }
}
pub struct AfterRequest;
#[async_trait]
impl DogAfterHook<Value, BusinessParams> for AfterRequest {
    async fn run(&self, ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        super::invoice_payments_schema::response(ctx)
    }
}
