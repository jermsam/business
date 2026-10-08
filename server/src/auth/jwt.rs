use crate::services::BusinessParams;
use anyhow::Result;
use dog_auth::{
    core::{
        AuthenticationBase, AuthenticationBuilder, AuthenticationParams, AuthenticationRequest,
        AuthenticationResult, AuthenticationStrategy,
    },
    JwtStrategy,
};
use dog_core::{DogError, HookContext};
use serde_json::Value;
use std::sync::Arc;
struct TenantJwt {
    inner: JwtStrategy<BusinessParams>,
    resolver: super::local::TypeDbUserResolver,
}
#[async_trait::async_trait]
impl AuthenticationStrategy<BusinessParams> for TenantJwt {
    async fn authenticate(
        &self,
        request: &AuthenticationRequest,
        params: &AuthenticationParams,
        ctx: &mut HookContext<Value, BusinessParams>,
        auth: &AuthenticationBase<BusinessParams>,
    ) -> Result<AuthenticationResult> {
        #[cfg(test)]
        let started = std::time::Instant::now();
        #[cfg(test)]
        crate::access::trace_enter("jwt-inner-start");
        let mut result = self.inner.authenticate(request, params, ctx, auth).await?;
        #[cfg(test)]
        crate::access::trace_stage("jwt-verify", started);
        #[cfg(test)]
        crate::access::trace_enter("jwt-inner-done");
        let id = result
            .pointer("/payload/sub")
            .and_then(Value::as_str)
            .ok_or_else(|| DogError::not_authenticated("Missing identity").into_anyhow())?
            .to_owned();
        let mut user = self
            .resolver
            .resolve_id(&id, &ctx.tenant.tenant_id.0)
            .await
            .map_err(|_| DogError::not_authenticated("Invalid identity").into_anyhow())?
            .ok_or_else(|| {
                DogError::not_authenticated("Invalid tenant membership").into_anyhow()
            })?;
        #[cfg(test)]
        crate::access::trace_enter("jwt-resolver-done");
        if let Some(map) = user.as_object_mut() {
            map.remove("password");
        }
        result["user"] = user;
        Ok(result)
    }
}
pub fn register_jwt(
    auth: &mut AuthenticationBuilder<BusinessParams>,
    state: Arc<crate::typedb::TypeDBState>,
) {
    auth.register(
        "jwt",
        Arc::new(TenantJwt {
            inner: JwtStrategy::new(),
            resolver: super::local::TypeDbUserResolver {
                adapter: dog_typedb::TypeDBAdapter::new(state),
            },
        }),
    );
}
