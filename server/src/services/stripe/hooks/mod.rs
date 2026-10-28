//! Security hooks shared by Stripe resource proxies. Credential storage and
//! resolution belong to auth-stripe; validation belongs to each resource.
pub(crate) mod request_safety;
mod resolve_credentials;
use crate::services::BusinessParams;
use dog_core::DogAppBuilder;
pub use resolve_credentials::ResolveCredentials;
use serde_json::Value;
use std::sync::Arc;

pub fn register_security_hooks(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    service: &str,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
) -> anyhow::Result<()> {
    builder.service_hooks(service, |h| {
        h.before_all(Arc::new(
            dog_auth::hooks::AuthenticateHook::new(auth, vec!["jwt".into()]).strict(),
        ));
        h.before_all(Arc::new(ResolveCredentials));
        h.before_all(Arc::new(request_safety::RequestSafety));
    });
    Ok(())
}
