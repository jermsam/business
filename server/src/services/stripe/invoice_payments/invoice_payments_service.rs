use super::super::adapters::stripe_adapter::{StripeAdapter, StripeState};
use super::invoice_payments_shared;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogService, ServiceCapabilities, TenantContext};
use serde_json::Value;
use std::sync::Arc;
pub struct InvoicePaymentsService {
    pub adapter: StripeAdapter,
}
impl InvoicePaymentsService {
    pub fn new(state: Arc<StripeState>) -> Self {
        Self {
            adapter: StripeAdapter::new(state, "/invoice_payments", "inpay_"),
        }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for InvoicePaymentsService {
    fn capabilities(&self) -> ServiceCapabilities {
        invoice_payments_shared::capabilities()
    }
    async fn find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        self.adapter._find(ctx, params).await
    }
    async fn get(&self, ctx: &TenantContext, id: &str, params: BusinessParams) -> Result<Value> {
        self.adapter._get(ctx, id, params).await
    }
}
