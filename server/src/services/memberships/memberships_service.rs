use crate::services::{workspace::WorkspaceService, BusinessParams};
use async_trait::async_trait;
use dog_core::{tenant::TenantContext, DogError, DogService, ServiceCapabilities};
use serde_json::Value;
pub struct MembershipsService(pub WorkspaceService);
impl MembershipsService {
    async fn scope(&self, ctx: &TenantContext, params: BusinessParams) -> anyhow::Result<String> {
        Ok(format!(
            "{} let $owner = biz_owner($u, $t, $now); $owner == true; ",
            self.0.scope(ctx, params).await?
        ))
    }
    fn one(mut rows: Vec<Value>) -> anyhow::Result<Value> {
        if rows.len() != 1 {
            return Err(
                DogError::not_found("Membership unavailable or operation not allowed")
                    .into_anyhow(),
            );
        }
        Ok(rows.remove(0))
    }
    fn target(id: &str) -> anyhow::Result<String> {
        let id = crate::services::records::records_schema::id(id)?;
        Ok(format!(r#"$person isa user, has biz_id "{id}"; "#))
    }
}
const MEMBERS: &str = r#"$person isa user, has biz_id $pid; $t has biz_id $tid; let $key = $tid + ":" + $pid;
 $m isa biz_membership, has biz_id == $key, links (tenant: $t, person: $person), has biz_role $role, has biz_state $state;"#;
const FETCH: &str =
    r#"fetch { "person_id": $person.biz_id, "role": $m.biz_role, "state": $m.biz_state };"#;
#[async_trait]
impl DogService<Value, BusinessParams> for MembershipsService {
    fn capabilities(&self) -> ServiceCapabilities {
        super::memberships_shared::capabilities()
    }
    async fn find(
        &self,
        ctx: &TenantContext,
        params: BusinessParams,
    ) -> anyhow::Result<Vec<Value>> {
        let scope = self.scope(ctx, params).await?;
        self.0.query(format!("{scope}{MEMBERS} select $person, $pid, $m; distinct; sort $pid; limit 100; {FETCH}"), false).await
    }
    async fn get(
        &self,
        ctx: &TenantContext,
        id: &str,
        params: BusinessParams,
    ) -> anyhow::Result<Value> {
        let scope = self.scope(ctx, params).await?;
        Self::one(
            self.0
                .query(
                    format!("{scope}{} {MEMBERS} {FETCH}", Self::target(id)?),
                    false,
                )
                .await?,
        )
    }
    async fn create(
        &self,
        ctx: &TenantContext,
        data: Value,
        params: BusinessParams,
    ) -> anyhow::Result<Value> {
        let data = super::memberships_schema::create(data)?;
        let scope = self.scope(ctx, params).await?;
        let query = format!(
            r#"{scope} {} $person has biz_id $pid; $t has biz_id $tid; let $key = $tid + ":" + $pid; not {{ $exists isa biz_membership, has biz_id == $key; }};
   select $t, $person, $key, $now; distinct;
   insert $m isa biz_membership, links (tenant: $t, person: $person), has biz_id == $key,
    has biz_role "{}", has biz_state "active", has biz_start == $now;
   {FETCH}"#,
            Self::target(&data.person_id)?,
            data.role
        );
        Self::one(self.0.query(query, true).await?)
    }
    async fn patch(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        data: Value,
        params: BusinessParams,
    ) -> anyhow::Result<Value> {
        let id = id.ok_or_else(|| DogError::bad_request("Person ID required").into_anyhow())?;
        let data = super::memberships_schema::change(data)?;
        let scope = self.scope(ctx, params).await?;
        let safety = if data.role == "owner" && data.state == "active" {
            ""
        } else {
            "let $other = biz_other_owner($person, $t, $now); $other == true;"
        };
        let query = format!(
            r#"{scope} {} {MEMBERS} {safety}
   select $t, $person, $m; distinct;
   update $m has biz_role "{}", has biz_state "{}";
   {FETCH}"#,
            Self::target(id)?,
            data.role,
            data.state
        );
        Self::one(self.0.query(query, true).await?)
    }
}
