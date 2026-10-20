use super::super::adapters::mercury_adapter::{MercuryAdapter, MercuryState, StoreKind};
use super::invoices_shared;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogService, ServiceCapabilities, TenantContext};
use serde_json::Value;
use std::sync::Arc;
pub struct InvoicesService {
    pub adapter: MercuryAdapter,
}
impl InvoicesService {
    pub fn new(state: Arc<MercuryState>) -> Self {
        Self {
            adapter: MercuryAdapter::new(state, StoreKind::Invoices),
        }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for InvoicesService {
    fn capabilities(&self) -> ServiceCapabilities {
        invoices_shared::capabilities()
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

    async fn custom(
        &self,
        ctx: &TenantContext,
        method: &str,
        data: Option<Value>,
        params: BusinessParams,
    ) -> Result<Value> {
        let data = data
            .ok_or_else(|| dog_core::DogError::bad_request("Invoice id required").into_anyhow())?;
        let id = data["id"]
            .as_str()
            .ok_or_else(|| dog_core::DogError::bad_request("Invoice id required").into_anyhow())?;
        match method {
            "cancel" => self.adapter._cancel(ctx, id, params).await,
            "pdf" => self.adapter._pdf(ctx, id, params).await,
            _ => {
                Err(dog_core::DogError::method_not_allowed("Unknown invoice method").into_anyhow())
            }
        }
    }
}
