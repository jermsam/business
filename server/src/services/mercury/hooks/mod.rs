//! Security hooks shared by Mercury resource proxies. Credential storage and
//! resolution belong to auth-mercury; validation belongs to each resource.
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
    });
    Ok(())
}
