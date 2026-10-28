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
        ServiceMethodKind::Remove,
    ])
}
pub fn register_hooks(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
) -> anyhow::Result<()> {
    super::super::hooks::register_security_hooks(builder, "stripe/invoice-items", auth)?;
    builder.service_hooks("stripe/invoice-items", |h| {
        h.before_all(Arc::new(super::invoice_items_hooks::BeforeRequest));
        h.after_all(Arc::new(super::invoice_items_hooks::AfterRequest));
    });
    Ok(())
}
