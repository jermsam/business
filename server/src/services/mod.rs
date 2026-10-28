use dog_core::DogAppBuilder;
use serde_json::Value;
use std::sync::Arc;
pub(crate) mod data;
pub mod mercury;
pub(crate) mod portal;
pub mod stripe;
mod types;
pub use types::*;
pub(crate) mod authentication;

pub fn configure(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<authentication::AuthService>,
    state: Arc<crate::typedb::TypeDBState>,
) -> anyhow::Result<()> {
    builder.register_service("authentication", auth);
    authentication::authentication_shared::register_hooks(builder)?;
    builder.register_service("data", Arc::new(data::DataService::new(state.clone())));
    let authentication: Arc<dog_auth::AuthenticationService<BusinessParams>> = builder
        .get("authentication")
        .ok_or_else(|| anyhow::anyhow!("Authentication service missing"))?;
    data::data_shared::register_hooks(builder, authentication.clone())?;
    if std::env::var("BILLING_ENABLED").as_deref() == Ok("true") {
        builder.register_service("portal", Arc::new(portal::PortalService::new(state)));
        portal::portal_shared::register_hooks(builder, authentication.clone())?;
    }
    if let Some(credentials) = mercury::auth_mercury::AuthMercuryState::from_env()? {
        let state = mercury::adapters::mercury_adapter::MercuryState::new(Arc::new(
            mercury::adapters::mercury_adapter::MercuryHttp::new()?,
        ));
        mercury::configure(
            builder,
            authentication.clone(),
            Arc::new(state),
            Arc::new(credentials),
        )?;
    }
    if let Some(credentials) = stripe::auth_stripe::AuthStripeState::from_env()? {
        let state = stripe::adapters::stripe_adapter::StripeState::new(Arc::new(
            stripe::adapters::stripe_adapter::StripeHttp::new()?,
        ));
        stripe::configure(
            builder,
            authentication,
            Arc::new(state),
            Arc::new(credentials),
        )?;
    }
    Ok(())
}
