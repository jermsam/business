use crate::services::BusinessParams;
use anyhow::Result;
use dog_auth::{core::AuthenticationBuilder, AuthenticationStrategy};
use dog_auth_local::{LocalEntityResolver, LocalStrategy};
use dog_core::HookContext;
use dog_typedb::TypeDBAdapter;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) struct TypeDbUserResolver {
    pub(crate) adapter: TypeDBAdapter,
}
pub(crate) fn tenant_match(tenant: &str) -> Result<String> {
    anyhow::ensure!(
        !tenant.is_empty()
            && tenant.len() <= 253
            && tenant
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b)),
        "Invalid tenant"
    );
    let attribute = if uuid::Uuid::parse_str(tenant).is_ok() {
        "biz_id"
    } else {
        "company_domain"
    };
    Ok(format!(
        r#"$t isa company, has {attribute} "{tenant}", has biz_id $tenant_id;"#
    ))
}
pub(crate) fn now() -> String {
    chrono::Utc::now()
        .naive_utc()
        .format("%Y-%m-%dT%H:%M:%S%.6f")
        .to_string()
}
pub(crate) fn lookup_query(email: &str, tenant: &str) -> Result<String> {
    // A deliberately restricted login identifier grammar. Never interpolate raw TypeQL.
    anyhow::ensure!(
        !email.is_empty()
            && email.len() <= 254
            && email.contains('@')
            && email
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b)),
        "Invalid email"
    );
    lookup_identity(&format!(r#"has email "{email}""#), tenant)
}
fn lookup_identity(identity: &str, tenant: &str) -> Result<String> {
    Ok(format!(
        r#"match $u isa user, {identity}, has biz_id $id, has password $password;
      {}
      select $u, $t, $id, $tenant_id, $password;
      match let $active = biz_account_active($u, $t); $active == true;
      let $member = biz_member($u, $t, {}); $member == true;
      fetch {{ "id": $id, "tenant_id": $tenant_id, "email": $u.email, "password": $password, "versions": [$u.auth_version] }};"#,
        tenant_match(tenant)?,
        now()
    ))
}
impl TypeDbUserResolver {
    pub(crate) async fn resolve_id(&self, id: &str, tenant: &str) -> Result<Option<Value>> {
        let id = uuid::Uuid::parse_str(id)?.to_string(); // Legacy email-subject tokens are invalidated.
        let query = lookup_identity(&format!(r#"has biz_id "{id}""#), tenant)?;
        entity_from_response(&self.adapter.read(json!({"query":query})).await?)
    }
}
fn entity_from_response(response: &Value) -> Result<Option<Value>> {
    let rows = response
        .pointer("/ok/answers")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Invalid user lookup response"))?;
    if rows.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(rows.len() == 1, "Ambiguous user lookup");
    let user = &rows[0]["data"];
    let email = user["email"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid user email"))?;
    let password = user["password"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid stored password"))?;
    let id = canonical_identity(user, "id")?;
    let tenant_id = canonical_identity(user, "tenant_id")?;
    Ok(Some(
        json!({"id":id,"tenant_id":tenant_id,"email":email,"password":password,"credential_version":user.pointer("/versions/0").and_then(Value::as_str).unwrap_or("initial")}),
    ))
}
fn canonical_identity(user: &Value, field: &str) -> Result<String> {
    let raw = user[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing stable identity"))?;
    let id = uuid::Uuid::parse_str(raw)?.to_string();
    anyhow::ensure!(id == raw, "Noncanonical identity");
    Ok(id)
}
#[async_trait::async_trait]
impl LocalEntityResolver<BusinessParams> for TypeDbUserResolver {
    async fn resolve_entity(
        &self,
        username: &str,
        ctx: &mut HookContext<Value, BusinessParams>,
    ) -> Result<Option<Value>> {
        let query = match lookup_query(username, &ctx.tenant.tenant_id.0) {
            Ok(query) => query,
            Err(_) => return Ok(None),
        };
        let response = self.adapter.read(json!({"query":query})).await?;
        entity_from_response(&response)
    }
}
pub fn register_local(
    auth: &mut AuthenticationBuilder<BusinessParams>,
    state: Arc<crate::typedb::TypeDBState>,
) {
    let strategy = LocalStrategy::new().with_entity_resolver(Arc::new(TypeDbUserResolver {
        adapter: TypeDBAdapter::new(state),
    }));
    auth.register(
        "local",
        Arc::new(VersionedLocal(strategy)) as Arc<dyn AuthenticationStrategy<BusinessParams>>,
    );
}
// Stamp the version observed by the password verifier, never a second lookup that
// could race a reset and bless an old password with a newer credential version.
struct VersionedLocal(LocalStrategy<BusinessParams>);
#[async_trait::async_trait]
impl AuthenticationStrategy<BusinessParams> for VersionedLocal {
    async fn authenticate(
        &self,
        request: &dog_auth::core::AuthenticationRequest,
        params: &dog_auth::core::AuthenticationParams,
        ctx: &mut HookContext<Value, BusinessParams>,
        auth: &dog_auth::core::AuthenticationBase<BusinessParams>,
    ) -> Result<dog_auth::core::AuthenticationResult> {
        let mut result = self.0.authenticate(request, params, ctx, auth).await?;
        let payload = json!({"sub":result["user"]["id"],"credential_version":result["user"]["credential_version"]});
        result["accessToken"] = json!(auth.create_access_token(payload, None).await?);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_rejects_injection_and_requires_tenant_membership() {
        assert!(lookup_query("x@example.com\"; delete $u;", "example.com").is_err());
        assert!(lookup_query("x@example.com", "x\"").is_err());
        assert!(lookup_query("x@example.com", "example.com")
            .unwrap()
            .contains("biz_member"));
    }
    #[test]
    fn missing_or_ambiguous_users_never_authenticate() {
        assert!(entity_from_response(&json!({"ok":{"answers":[]}}))
            .unwrap()
            .is_none());
        assert!(entity_from_response(&json!({"ok":{"answers":[{},{}]}})).is_err());
        let result = entity_from_response(
            &json!({"ok":{"answers":[{"data":{"id":"00000000-0000-4000-8000-000000000001", "tenant_id":"00000000-0000-4000-8000-000000000002", "email":"a@b.com","password":"hash"}}]}}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(result["id"], "00000000-0000-4000-8000-000000000001");
        assert!(entity_from_response(&json!({"ok":{"answers":[{"data":{"id":"bad", "tenant_id":"bad", "email":"a@b.com", "password":"hash"}}]}})).is_err());
    }
}
