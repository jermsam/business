use crate::services::BusinessParams;
use dog_core::{DogAppBuilder, ServiceCapabilities, ServiceMethodKind};
use serde_json::Value;
use std::sync::Arc;
pub fn capabilities() -> ServiceCapabilities {
    ServiceCapabilities::from_methods(vec![ServiceMethodKind::Get])
}
pub fn register_hooks(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
) -> anyhow::Result<()> {
    super::super::hooks::register_security_hooks(builder, "stripe/mandates", auth)?;
    builder.service_hooks("stripe/mandates", |h| {
        h.before_all(Arc::new(super::mandates_hooks::BeforeRequest));
        h.after_all(Arc::new(super::mandates_hooks::AfterRequest));
    });
    Ok(())
}
