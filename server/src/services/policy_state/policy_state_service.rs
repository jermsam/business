use crate::services::{workspace::WorkspaceService, BusinessParams};
use async_trait::async_trait;
use dog_core::{tenant::TenantContext, DogError, DogService, ServiceCapabilities};
use serde_json::Value;
pub enum Target {
    Grant,
    Entitlement,
    TeamMembership,
}
pub struct PolicyStateService(pub WorkspaceService, pub Target);
#[async_trait]
impl DogService<Value, BusinessParams> for PolicyStateService {
    fn capabilities(&self) -> ServiceCapabilities {
        super::policy_state_shared::capabilities()
    }
    async fn patch(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        data: Value,
        params: BusinessParams,
    ) -> anyhow::Result<Value> {
        let id = super::policy_state_schema::id(
            id.ok_or_else(|| DogError::bad_request("Policy ID required").into_anyhow())?,
        )?;
        let data = super::policy_state_schema::parse(data)?;
        let scope = self.0.scope(ctx, params).await?;
        // An organization owner may disable access, but cannot manufacture a paid entitlement.
        if matches!(self.1, Target::Entitlement) && data.state == "active" {
            return Err(DogError::forbidden(
                "Entitlement activation requires trusted provisioning",
            )
            .into_anyhow());
        }
        let target=match self.1 {
   Target::Grant=>"$target isa biz_grant, links (tenant:$t);",
   Target::Entitlement=>"$target isa biz_entitlement, links (tenant:$t);",
   Target::TeamMembership=>"$target isa biz_team_member, links (team:$team); (tenant:$t,team:$team) isa biz_team_owner;",
  };
        let query = format!(
            r#"{scope} let $owner=biz_owner($u,$t,$now); $owner==true;
   {target} $target has biz_id "{id}";
   select $t,$target; distinct; update $target has biz_state "{}";
   fetch {{"id":$target.biz_id,"state":$target.biz_state}};"#,
            data.state
        );
        let mut rows = self.0.query(query, true).await?;
        if rows.len() != 1 {
            return Err(DogError::not_found("Policy unavailable").into_anyhow());
        }
        Ok(rows.remove(0))
    }
}
