use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogAfterHook, DogApp, HookContext};
use serde_json::Value;
use std::sync::Arc;

pub struct AfterRead;

#[async_trait]
impl DogAfterHook<Value, BusinessParams> for AfterRead {
    async fn run(&self, _ctx: &mut HookContext<Value, BusinessParams>) -> Result<()> {
        Ok(())
    }
}

pub fn global_hook(app: &DogApp<Value, BusinessParams>) -> Result<()> {
    // Global hooks run at the app level, before any service-specific hooks
    // The issue: before_all runs BEFORE authentication
    // Solution: We need service-level hooks that run AFTER auth
    // For now, keep global after_all for logging, but multi-tenancy is in service hooks
    app.hooks(|h| {
        h.after_all(Arc::new(AfterRead));
    });

    Ok(())
}
