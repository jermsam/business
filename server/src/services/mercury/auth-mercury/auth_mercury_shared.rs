use crate::services::BusinessParams;
use dog_core::{DogAppBuilder, ServiceCapabilities, ServiceMethodKind};
use serde_json::Value;
use std::sync::Arc;
pub fn capabilities() -> ServiceCapabilities {
    ServiceCapabilities::from_methods(vec![ServiceMethodKind::Custom("resolve")])
}
pub fn register_hooks(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
) -> anyhow::Result<()> {
    builder.service_hooks("auth-mercury", |h| {
        h.before_all(Arc::new(
            dog_auth::hooks::AuthenticateHook::new(auth, vec!["jwt".into()]).strict(),
        ));
        h.before_all(Arc::new(super::auth_mercury_hooks::BeforeResolve));
        h.after_all(Arc::new(super::auth_mercury_hooks::AfterResolve));
    });
    Ok(())
}
