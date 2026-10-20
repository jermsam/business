//! Cryptography and credential verification stay in DogRS authentication, not
//! in public database queries. Membership eligibility is a TypeDB function.
use crate::{services::BusinessParams, typedb::TypeDBState};
use anyhow::{ensure, Result};
use async_trait::async_trait;
use dog_auth::{
    core::{
        AuthenticationBase, AuthenticationParams, AuthenticationRequest, AuthenticationResult,
        AuthenticationStrategy,
    },
    AuthOptions, AuthStrategy, AuthenticationService, JwtStrategy,
};
use dog_auth_local::{LocalEntityResolver, LocalStrategy};
use dog_core::{DogAppBuilder, DogError, HookContext};
use dog_typedb::TypeDBAdapter;
use serde_json::{json, Value};
use std::sync::Arc;

struct Resolver {
    adapter: TypeDBAdapter,
}
impl Resolver {
    async fn lookup(&self, attribute: &str, value: &str, tenant: &str) -> Result<Option<Value>> {
        ensure!(
            matches!(attribute, "email" | "identity_id"),
            "Invalid credential selector"
        );
        let q = |v: &str| serde_json::to_string(v).unwrap();
        let query = format!(
            r#"match
          $i isa person, has {attribute} {}, has identity_id $id;
          $t isa tenant, has tenant_id {}, has tenant_id $tenant;
          let $active=active_member($i,$t,{}); $active==true;
          fetch {{"id":$id,"tenant_id":$tenant,"email":$i.email,
             "password":$i.password_hash,"credential_version":$i.credential_version}};"#,
            q(value),
            q(tenant),
            now()
        );
        let response = self.adapter.read(json!({"query":query})).await?;
        let rows = response
            .pointer("/ok/answers")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("Invalid credential result"))?;
        ensure!(rows.len() <= 1, "Ambiguous credential identity");
        Ok(rows.first().map(|r| r["data"].clone()))
    }
}
#[async_trait]
impl LocalEntityResolver<BusinessParams> for Resolver {
    async fn resolve_entity(
        &self,
        email: &str,
        ctx: &mut HookContext<Value, BusinessParams>,
    ) -> Result<Option<Value>> {
        self.lookup(
            "email",
            &email.trim().to_ascii_lowercase(),
            &ctx.tenant.tenant_id.0,
        )
        .await
    }
}
struct Local(LocalStrategy<BusinessParams>);
#[async_trait]
impl AuthenticationStrategy<BusinessParams> for Local {
    async fn authenticate(
        &self,
        request: &AuthenticationRequest,
        params: &AuthenticationParams,
        ctx: &mut HookContext<Value, BusinessParams>,
        auth: &AuthenticationBase<BusinessParams>,
    ) -> Result<AuthenticationResult> {
        let mut result = self.0.authenticate(request, params, ctx, auth).await?;
        result["accessToken"] = json!(auth.create_access_token(json!({
            "sub":result["user"]["id"],"credential_version":result["user"]["credential_version"]
        }),None).await?);
        Ok(result)
    }
}
struct Jwt {
    inner: JwtStrategy<BusinessParams>,
    resolver: Resolver,
}
#[async_trait]
impl AuthenticationStrategy<BusinessParams> for Jwt {
    async fn authenticate(
        &self,
        request: &AuthenticationRequest,
        params: &AuthenticationParams,
        ctx: &mut HookContext<Value, BusinessParams>,
        auth: &AuthenticationBase<BusinessParams>,
    ) -> Result<AuthenticationResult> {
        let mut result = self.inner.authenticate(request, params, ctx, auth).await?;
        let id = result
            .pointer("/payload/sub")
            .and_then(Value::as_str)
            .ok_or_else(|| DogError::not_authenticated("Missing identity").into_anyhow())?;
        let mut person = self
            .resolver
            .lookup("identity_id", id, &ctx.tenant.tenant_id.0)
            .await?
            .ok_or_else(|| {
                DogError::not_authenticated("Inactive identity or membership").into_anyhow()
            })?;
        if result
            .pointer("/payload/credential_version")
            .and_then(Value::as_str)
            != person.get("credential_version").and_then(Value::as_str)
        {
            return Err(
                DogError::not_authenticated("Credentials changed; sign in again").into_anyhow(),
            );
        }
        person.as_object_mut().unwrap().remove("password");
        result["user"] = person;
        Ok(result)
    }
}
pub fn strategies(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    state: Arc<TypeDBState>,
) -> Result<Arc<dog_auth::AuthServiceAdapter<BusinessParams>>> {
    let mut opts = AuthOptions {
        strategies: vec![AuthStrategy::Jwt, AuthStrategy::Custom("local".into())],
        ..Default::default()
    };
    opts.jwt.secret = builder.get("auth.jwt.secret");
    opts.entity = Some("user".into());
    let mut auth = AuthenticationService::builder(builder, Some(opts))?.with_token_store(Arc::new(
        crate::auth::token_store::TypeDbTokenStore::new(state.clone()),
    ));
    auth.register(
        "local",
        Arc::new(Local(LocalStrategy::new().with_entity_resolver(Arc::new(
            Resolver {
                adapter: TypeDBAdapter::new(state.clone()),
            },
        )))),
    );
    auth.register(
        "jwt",
        Arc::new(Jwt {
            inner: JwtStrategy::new(),
            resolver: Resolver {
                adapter: TypeDBAdapter::new(state),
            },
        }),
    );
    let service = Arc::new(AuthenticationService::new(Arc::new(auth.build())));
    Ok(AuthenticationService::install(builder, service))
}

fn now() -> String {
    chrono::Utc::now()
        .naive_utc()
        .format("%Y-%m-%dT%H:%M:%S%.6f")
        .to_string()
}
pub mod token_store;
