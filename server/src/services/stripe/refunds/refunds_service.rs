use super::super::adapters::stripe_adapter::{StripeAdapter, StripeState};
use super::refunds_shared;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogService, ServiceCapabilities, TenantContext};
use serde_json::Value;
use std::sync::Arc;
pub struct RefundsService {
    pub adapter: StripeAdapter,
}
impl RefundsService {
    pub fn new(state: Arc<StripeState>) -> Self {
        Self {
            adapter: StripeAdapter::new(state, "/refunds", "re_"),
        }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for RefundsService {
    fn capabilities(&self) -> ServiceCapabilities {
        refunds_shared::capabilities()
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
            "cancel" => {
                self.adapter
                    ._action(ctx, &id, "cancel", body, params, false)
                    .await
            }
            _ => Err(
                dog_core::DogError::method_not_allowed("Unsupported resource method").into_anyhow(),
            ),
        }
    }
}
