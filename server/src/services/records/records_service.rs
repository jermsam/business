use super::{records_schema, records_shared};
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

/// Private records: selected tenant AND verified creator must match every operation.
/// Team sharing must be implemented explicitly; membership alone does not grant it.
pub struct RecordsService {
    adapter: TypeDBAdapter,
    state: Arc<TypeDBState>,
    auth: Arc<AuthService>,
}
impl RecordsService {
    pub fn new(state: Arc<TypeDBState>, auth: Arc<AuthService>) -> Self {
        Self {
            adapter: TypeDBAdapter::new(state.clone()),
            state,
            auth,
        }
    }
    async fn scope(&self, ctx: &TenantContext, params: BusinessParams) -> Result<String> {
        // Never trust caller-supplied authenticated/auth_result/provider flags.
        let token = dog_auth::core::extract_bearer_token(&params.headers)
            .ok_or_else(|| DogError::not_authenticated("Bearer token required").into_anyhow())?;
        let verified = self
            .auth
            .create(ctx, json!({"strategy":"jwt", "accessToken":token}), params)
            .await?;
        let id = verified
            .pointer("/user/id")
            .and_then(Value::as_str)
            .ok_or_else(|| DogError::not_authenticated("Missing identity").into_anyhow())?;
        let tenant = verified
            .pointer("/user/tenant_id")
            .and_then(Value::as_str)
            .ok_or_else(|| DogError::not_authenticated("Missing tenant").into_anyhow())?;
        let id = uuid::Uuid::parse_str(id)?;
        let tenant = uuid::Uuid::parse_str(tenant)?;
        Ok(format!(
            r#"match $u isa user, has biz_id "{id}";
            $t isa company, has biz_id "{tenant}", has biz_revision $revision;
            let $now = {};
            select $u, $t, $now;
            match let $member = biz_private_authorized($u, $t, $now); $member == true;
            "#,
            crate::auth::local::now()
        ))
    }

    fn record_match(_ctx: &TenantContext, id: Option<&str>) -> Result<String> {
        let id_clause = match id {
            Some(id) => format!(r#", has record_id "{}""#, records_schema::id(id)?),
            None => String::new(),
        };
        Ok(format!(
            r#"$u has biz_id $owner; $t has biz_id $tenant;
            $r isa business_record, has biz_tenant_id == $tenant, has biz_person_id == $owner, has record_id $rid, has name $name{id_clause}; "#
        ))
    }

    async fn query(&self, query: String, write: bool) -> Result<Vec<Value>> {
        if write {
            return crate::access::write_one(&self.state, &query).await;
        }
        let result = self.adapter.read(json!({"query":query})).await;
        #[cfg(test)]
        if let Err(error) = &result {
            eprintln!("Private record query failed: {error:#}");
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
impl DogService<Value, BusinessParams> for RecordsService {
    fn capabilities(&self) -> ServiceCapabilities {
        records_shared::capabilities()
    }
    async fn find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        let scope = self.scope(ctx, params).await?;
        self.query(
            format!(
                "{scope}{} select $rid, $name; distinct; sort $rid; limit 100; {FETCH}",
                Self::record_match(ctx, None)?
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
                    Self::record_match(ctx, Some(id))?
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
        let data = records_schema::input(data)?;
        let id = uuid::Uuid::new_v4();
        Self::one(self.query(format!(r#"{scope} $u has biz_id $owner, has email $email; $t has biz_id $tenant, has company_domain $domain;
            select $t, $owner, $tenant, $email, $domain; distinct;
            insert $r isa business_record, has record_id "{id}", has biz_person_id == $owner, has biz_tenant_id == $tenant, has $domain, has $email, has name "{}";
            fetch {{ "id": $r.record_id, "name": $r.name }};"#,data.name),true).await?)
    }

    async fn update(
        &self,
        ctx: &TenantContext,
        id: &str,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        let scope = self.scope(ctx, params).await?;
        let data = records_schema::input(data)?;
        Self::one(
            self.query(
                format!(
                    r#"{scope}{} select $t, $r, $rid; distinct;
            update $r has name "{}";
            fetch {{ "id": $rid, "name": $r.name }};"#,
                    Self::record_match(ctx, Some(id))?,
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
                    "{scope}{} select $t, $r, $rid, $name; distinct; delete $r; {FETCH}",
                    Self::record_match(ctx, Some(id))?
                ),
                true,
            )
            .await?,
        )
    }
}
