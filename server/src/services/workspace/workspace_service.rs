use super::{workspace_schema, workspace_shared};
use crate::{
    services::{authentication::AuthService, BusinessParams},
    typedb::TypeDBState,
};
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{tenant::TenantContext, DogError, DogService, ServiceCapabilities};
use dog_typedb::TypeDBAdapter;
use serde_json::{json, Value};
use std::sync::Arc;

/// Shared project records: all operations call the tenant-scoped TypeQL policy.
pub struct WorkspaceService {
    adapter: TypeDBAdapter,
    state: Arc<TypeDBState>,
    auth: Arc<AuthService>,
}
impl WorkspaceService {
    pub fn new(state: Arc<TypeDBState>, auth: Arc<AuthService>) -> Self {
        Self {
            adapter: TypeDBAdapter::new(state.clone()),
            state,
            auth,
        }
    }
    pub(crate) async fn scope(
        &self,
        ctx: &TenantContext,
        params: BusinessParams,
    ) -> Result<String> {
        // Never trust caller-supplied authenticated/auth_result/provider flags.
        let token = dog_auth::core::extract_bearer_token(&params.headers)
            .ok_or_else(|| DogError::not_authenticated("Bearer token required").into_anyhow())?;
        #[cfg(test)]
        let started = std::time::Instant::now();
        let verified = self
            .auth
            .create(ctx, json!({"strategy":"jwt", "accessToken":token}), params)
            .await?;
        #[cfg(test)]
        crate::access::trace_stage("workspace-auth", started);
        let id = verified
            .pointer("/user/id")
            .and_then(Value::as_str)
            .ok_or_else(|| DogError::not_authenticated("Missing identity").into_anyhow())?;
        let tenant = verified
            .pointer("/user/tenant_id")
            .and_then(Value::as_str)
            .ok_or_else(|| DogError::not_authenticated("Missing tenant").into_anyhow())?;
        let credential = verified
            .pointer("/user/credential_version")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DogError::not_authenticated("Credential version required").into_anyhow()
            })?;
        let credential = crate::services::billing::billing_schema::quoted(credential);
        let id = uuid::Uuid::parse_str(id)?;
        let tenant = uuid::Uuid::parse_str(tenant)?;
        Ok(format!(
            r#"match $u isa user, has biz_id "{id}"; not {{$u has auth_version $changed; $changed != {credential};}};
            $t isa company, has biz_id "{tenant}", has biz_revision $revision;
            let $now = {};
            select $u, $t, $now;
            match
            "#,
            crate::auth::local::now()
        ))
    }

    fn record_match(id: Option<&str>, action: &str) -> Result<String> {
        let id_clause = match id {
            Some(id) => format!(r#", has biz_id "{}""#, workspace_schema::id(id)?),
            None => String::new(),
        };
        Ok(format!(
            r#"$r isa biz_record, has biz_id $rid, has name $name{id_clause};
            (record: $r, project: $project) isa biz_record_owner;
            let $allowed = biz_authorized($u, $t, $project, "{action}", $now); $allowed == true; "#
        ))
    }
    pub(crate) async fn query(&self, query: String, write: bool) -> Result<Vec<Value>> {
        if write {
            return crate::access::write_one(&self.state, &query).await;
        }
        let result = self.adapter.read(json!({"query":query})).await;
        #[cfg(test)]
        if let Err(error) = &result {
            eprintln!("Workspace query failed: {error:#}");
        }
        let result = result?;
        let rows = result
            .pointer("/ok/answers")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("Invalid database response"))?;
        Ok(rows.iter().map(|r| r["data"].clone()).collect())
    }
    fn one(mut rows: Vec<Value>) -> Result<Value> {
        if rows.is_empty() {
            return Err(DogError::not_found("Record not found").into_anyhow());
        }
        anyhow::ensure!(rows.len() == 1, "Ambiguous record result");
        Ok(rows.remove(0))
    }
}
const FETCH: &str = r#"fetch { "id": $rid, "name": $name };"#;
#[async_trait]
impl DogService<Value, BusinessParams> for WorkspaceService {
    fn capabilities(&self) -> ServiceCapabilities {
        workspace_shared::capabilities()
    }
    async fn find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        let scope = self.scope(ctx, params).await?;
        self.query(
            format!(
                "{scope}{} select $rid, $name; distinct; sort $rid; limit 100; {FETCH}",
                Self::record_match(None, "read")?
            ),
            false,
        )
        .await
    }
    async fn get(&self, ctx: &TenantContext, id: &str, params: BusinessParams) -> Result<Value> {
        let scope = self.scope(ctx, params).await?;
        Self::one(
            self.query(
                format!(
                    "{scope}{} select $rid, $name; distinct; {FETCH}",
                    Self::record_match(Some(id), "read")?
                ),
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
    ) -> Result<Value> {
        let scope = self.scope(ctx, params).await?;
        let (name, project) = workspace_schema::create(data)?;
        let id = uuid::Uuid::new_v4();
        Self::one(
            self.query(
                format!(
                    r#"{scope}
            $project isa biz_project, has biz_id "{project}";
            let $allowed = biz_authorized($u, $t, $project, "create", $now); $allowed == true;
            select $t, $u, $project; distinct;
            insert $r isa biz_record, has biz_id "{id}", has name "{name}";
            (record: $r, project: $project, creator: $u) isa biz_record_owner;
            fetch {{ "id": $r.biz_id, "name": $r.name }};"#
                ),
                true,
            )
            .await?,
        )
    }
    async fn update(
        &self,
        ctx: &TenantContext,
        id: &str,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        let scope = self.scope(ctx, params).await?;
        let data = workspace_schema::input(data)?;
        Self::one(
            self.query(
                format!(
                    r#"{scope}{} select $t, $r, $rid; distinct;
            update $r has name "{}";
            fetch {{ "id": $rid, "name": $r.name }};"#,
                    Self::record_match(Some(id), "update")?,
                    data.name
                ),
                true,
            )
            .await?,
        )
    }
    async fn patch(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        self.update(
            ctx,
            id.ok_or_else(|| DogError::bad_request("Record ID required").into_anyhow())?,
            data,
            params,
        )
        .await
    }
    async fn remove(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        params: BusinessParams,
    ) -> Result<Value> {
        let scope = self.scope(ctx, params).await?;
        let id = id.ok_or_else(|| DogError::bad_request("Record ID required").into_anyhow())?;
        Self::one(
            self.query(
                format!(
                    "{scope}{} $link isa biz_record_owner, links (record: $r); select $t, $r, $link, $rid, $name; distinct; delete $link; delete $r; {FETCH}",
                    Self::record_match(Some(id), "delete")?
                ),
                true,
            )
            .await?,
        )
    }
}
