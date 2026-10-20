use super::data_shared;
use crate::{services::BusinessParams, typedb::TypeDBState};
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{tenant::TenantContext, DogError, DogService, ServiceCapabilities};
use dog_typedb::isolation::SchemaPolicyAdapter;
use serde_json::Value;
use std::sync::Arc;

pub struct DataService {
    adapter: SchemaPolicyAdapter,
}
impl DataService {
    pub fn new(state: Arc<TypeDBState>) -> Self {
        Self {
            adapter: SchemaPolicyAdapter::new(state),
        }
    }
}
#[async_trait]
impl DogService<Value, BusinessParams> for DataService {
    fn capabilities(&self) -> ServiceCapabilities {
        data_shared::capabilities()
    }
    async fn custom(
        &self,
        _ctx: &TenantContext,
        method: &str,
        data: Option<Value>,
        params: BusinessParams,
    ) -> Result<Value> {
        if !matches!(method, "read" | "write") {
            return Err(
                DogError::method_not_allowed("Only read and write are supported").into_anyhow(),
            );
        }
        let scope = params.inner.schema_policy_scope.as_deref().ok_or_else(|| {
            DogError::not_authenticated("Authenticated scope required").into_anyhow()
        })?;
        let data =
            data.ok_or_else(|| DogError::bad_request("Missing request body").into_anyhow())?;
        match method {
            "read" => self.adapter.read(scope, data).await,
            "write" => self.adapter.write(scope, data).await,
            _ => unreachable!(),
        }
    }
}
