use super::super::adapters::stripe_adapter::{StripeAdapter, StripeState};
use super::invoices_shared;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogService, ServiceCapabilities, TenantContext};
use serde_json::Value;
use std::sync::Arc;
pub struct InvoicesService {
    pub adapter: StripeAdapter,
}
impl InvoicesService {
    pub fn new(state: Arc<StripeState>) -> Self {
        Self {
            adapter: StripeAdapter::new(state, "/invoices", "in_"),
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
    async fn patch(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        self.adapter._patch(ctx, id, data, params).await
    }
    async fn remove(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        params: BusinessParams,
    ) -> Result<Value> {
        self.adapter._remove(ctx, id, params).await
    }
    async fn custom(
        &self,
        ctx: &TenantContext,
        method: &str,
        data: Option<Value>,
        params: BusinessParams,
    ) -> Result<Value> {
        let mut body = data
            .ok_or_else(|| dog_core::DogError::bad_request("Resource id required").into_anyhow())?;
        let id = body["id"]
            .as_str()
            .ok_or_else(|| dog_core::DogError::bad_request("Resource id required").into_anyhow())?
            .to_string();
        body.as_object_mut()
            .ok_or_else(|| dog_core::DogError::bad_request("Send an object").into_anyhow())?
            .remove("id");
        match method {
            "finalize" => {
                self.adapter
                    ._action(ctx, &id, "finalize", body, params, false)
                    .await
            }
            "pay" => {
                self.adapter
                    ._action(ctx, &id, "pay", body, params, false)
                    .await
            }
            "send" => {
                self.adapter
                    ._action(ctx, &id, "send", body, params, false)
                    .await
            }
            "void" => {
                self.adapter
                    ._action(ctx, &id, "void", body, params, false)
                    .await
            }
            "mark_uncollectible" => {
                self.adapter
                    ._action(ctx, &id, "mark_uncollectible", body, params, false)
                    .await
            }
            "attach_payment" => {
                self.adapter
                    ._action(ctx, &id, "attach_payment", body, params, false)
                    .await
            }
            _ => Err(
                dog_core::DogError::method_not_allowed("Unsupported resource method").into_anyhow(),
            ),
        }
    }
}
