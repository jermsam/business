use crate::services::workspace::WorkspaceService;
use crate::services::BusinessParams;
use async_trait::async_trait;
use dog_core::{tenant::TenantContext, DogService, ServiceCapabilities};
use serde_json::Value;
pub struct AppsService(pub WorkspaceService);
#[async_trait]
impl DogService<Value, BusinessParams> for AppsService {
    fn capabilities(&self) -> ServiceCapabilities {
        super::apps_shared::capabilities()
    }
    async fn find(
        &self,
        ctx: &TenantContext,
        params: BusinessParams,
    ) -> anyhow::Result<Vec<Value>> {
        let scope = self.0.scope(ctx, params).await?;
        self.0
            .query(
                format!(
                    r#"{scope}
   (tenant: $t, product: $product, project: $project) isa biz_project_owner;
   let $allowed = biz_authorized($u, $t, $project, "read", $now); $allowed == true;
   $product has biz_id $id, has name $name;
   select $id, $name; distinct; sort $id; limit 100;
   fetch {{ "id": $id, "name": $name }};"#
                ),
                false,
            )
            .await
    }
    async fn get(
        &self,
        ctx: &TenantContext,
        id: &str,
        params: BusinessParams,
    ) -> anyhow::Result<Value> {
        let id = super::apps_schema::id(id)?;
        let scope = self.0.scope(ctx, params).await?;
        let projects = self
            .0
            .query(
                format!(
                    r#"{scope}
            $product isa biz_product, has biz_id "{id}";
            (tenant: $t, product: $product, project: $project) isa biz_project_owner;
            let $allowed = biz_authorized($u, $t, $project, "read", $now); $allowed == true;
            $project has biz_id $project_id, has name $name;
            select $project_id, $name; distinct; sort $project_id; limit 100;
            fetch {{ "id": $project_id, "name": $name }};"#
                ),
                false,
            )
            .await?;
        if projects.is_empty() {
            return Err(dog_core::DogError::not_found("Product not found").into_anyhow());
        }
        Ok(serde_json::json!({"id": id, "projects": projects}))
    }
}
