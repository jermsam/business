use super::super::adapters::mercury_adapter::{MercuryAdapter, MercuryState, StoreKind};
use super::transactions_shared;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogService, ServiceCapabilities, TenantContext};
use serde_json::Value;
use std::sync::Arc;
pub struct TransactionsService {
    pub adapter: MercuryAdapter,
}
impl TransactionsService {
    pub fn new(state: Arc<MercuryState>) -> Self {
        Self {
            adapter: MercuryAdapter::new(state, StoreKind::Transactions),
        }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for TransactionsService {
    fn capabilities(&self) -> ServiceCapabilities {
        transactions_shared::capabilities()
    }

    async fn find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        self.adapter._find(ctx, params).await
    }
    async fn get(&self, ctx: &TenantContext, id: &str, params: BusinessParams) -> Result<Value> {
        self.adapter._get(ctx, id, params).await
    }
}
