use super::super::adapters::mercury_adapter::{MercuryAdapter, MercuryState, StoreKind};
use super::customers_shared;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogService, ServiceCapabilities, TenantContext};
use serde_json::Value;
use std::sync::Arc;
pub struct CustomersService {
    pub adapter: MercuryAdapter,
}
impl CustomersService {
    pub fn new(state: Arc<MercuryState>) -> Self {
        Self {
            adapter: MercuryAdapter::new(state, StoreKind::Customers),
        }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for CustomersService {
    fn capabilities(&self) -> ServiceCapabilities {
        customers_shared::capabilities()
    }

    async fn find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        self.adapter._find(ctx, params).await
    }
    async fn get(&self, ctx: &TenantContext, id: &str, params: BusinessParams) -> Result<Value> {
        self.adapter._get(ctx, id, params).await
    }

    async fn create(
        &self,
        ctx: &TenantContext,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        self.adapter._create(ctx, data, params).await
    }
    async fn update(
        &self,
        ctx: &TenantContext,
        id: &str,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        self.adapter._update(ctx, id, data, params).await
    }

    async fn remove(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        params: BusinessParams,
    ) -> Result<Value> {
        self.adapter._remove(ctx, id, params).await
    }
}
