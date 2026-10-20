use super::super::adapters::stripe_adapter::{StripeAdapter, StripeState};
use super::charges_shared;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogService, ServiceCapabilities, TenantContext};
use serde_json::Value;
use std::sync::Arc;
pub struct ChargesService {
    pub adapter: StripeAdapter,
}
impl ChargesService {
    pub fn new(state: Arc<StripeState>) -> Self {
        Self {
            adapter: StripeAdapter::new(state, "/charges", "ch_"),
        }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for ChargesService {
    fn capabilities(&self) -> ServiceCapabilities {
        charges_shared::capabilities()
    }
    async fn find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        self.adapter._find(ctx, params).await
    }
    async fn get(&self, ctx: &TenantContext, id: &str, params: BusinessParams) -> Result<Value> {
        self.adapter._get(ctx, id, params).await
    }
    async fn patch(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        self.adapter._patch(ctx, id, data, params).await
    }
}
