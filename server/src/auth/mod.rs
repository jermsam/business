use crate::services::BusinessParams;
use anyhow::Result;
use dog_auth::{AuthOptions, AuthStrategy, AuthenticationService};
use dog_core::DogAppBuilder;
use serde_json::Value;
use std::sync::Arc;
pub mod jwt;
pub mod local;
pub mod token_store;

pub fn strategies(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
) -> Result<Arc<dog_auth::AuthServiceAdapter<BusinessParams>>> {
    let mut opts = AuthOptions {
        strategies: vec![AuthStrategy::Jwt, AuthStrategy::Custom("local".into())],
        ..Default::default()
    };
    opts.jwt.secret = builder.get("auth.jwt.secret");
    opts.entity = Some("user".into());
    let state = builder
        .get::<Arc<crate::typedb::TypeDBState>>("typedb")
        .ok_or_else(|| anyhow::anyhow!("TypeDB not initialized"))?;
    let mut auth = AuthenticationService::builder(builder, Some(opts))?
        .with_token_store(Arc::new(token_store::TypeDbTokenStore::new(state.clone())));
    jwt::register_jwt(&mut auth, state.clone());
    local::register_local(&mut auth, state);
    let service = Arc::new(AuthenticationService::new(Arc::new(auth.build())));
    Ok(AuthenticationService::install(builder, service))
}
