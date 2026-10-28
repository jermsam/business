use super::super::adapters::stripe_adapter::{StripeAdapter, StripeState};
use super::mandates_shared;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogService, ServiceCapabilities, TenantContext};
use serde_json::Value;
use std::sync::Arc;
pub struct MandatesService {
    pub adapter: StripeAdapter,
}
impl MandatesService {
    pub fn new(state: Arc<StripeState>) -> Self {
        Self {
            adapter: StripeAdapter::new(state, "/mandates", "mandate_"),
        }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for MandatesService {
    fn capabilities(&self) -> ServiceCapabilities {
        mandates_shared::capabilities()
    }
    async fn get(&self, ctx: &TenantContext, id: &str, params: BusinessParams) -> Result<Value> {
        self.adapter._get(ctx, id, params).await
    }
}
