use crate::services::BusinessParams;
use dog_core::{DogAppBuilder, ServiceCapabilities, ServiceMethodKind};
use serde_json::Value;
use std::sync::Arc;
pub fn capabilities() -> ServiceCapabilities {
    ServiceCapabilities::from_methods(vec![
        ServiceMethodKind::Find,
        ServiceMethodKind::Get,
        ServiceMethodKind::Create,
        ServiceMethodKind::Update,
        ServiceMethodKind::Custom("cancel"),
        ServiceMethodKind::Custom("pdf"),
    ])
}
pub fn register_hooks(
    app: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
) -> anyhow::Result<()> {
    super::super::hooks::register_security_hooks(app, "mercury/invoices", auth)?;
    app.service_hooks("mercury/invoices", |h| {
        h.before_all(Arc::new(super::invoices_hooks::BeforeRequest));
        h.after_all(Arc::new(super::invoices_hooks::AfterRequest));
    });
    Ok(())
}
