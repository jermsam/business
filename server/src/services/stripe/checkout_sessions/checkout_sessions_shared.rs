use crate::services::BusinessParams;
use dog_core::{DogAppBuilder, ServiceCapabilities, ServiceMethodKind};
use serde_json::Value;
use std::sync::Arc;
pub fn capabilities() -> ServiceCapabilities {
    ServiceCapabilities::from_methods(vec![
        ServiceMethodKind::Find,
        ServiceMethodKind::Get,
        ServiceMethodKind::Create,
        ServiceMethodKind::Patch,
        ServiceMethodKind::Custom("expire"),
        ServiceMethodKind::Custom("line_items"),
    ])
}
pub fn register_hooks(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
) -> anyhow::Result<()> {
    super::super::hooks::register_security_hooks(builder, "stripe/checkout-sessions", auth)?;
    builder.service_hooks("stripe/checkout-sessions", |h| {
        h.before_all(Arc::new(super::checkout_sessions_hooks::BeforeRequest));
        h.after_all(Arc::new(super::checkout_sessions_hooks::AfterRequest));
    });
    Ok(())
}
