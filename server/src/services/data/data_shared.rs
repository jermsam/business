use crate::services::BusinessParams;
use dog_core::{DogAppBuilder, ServiceCapabilities, ServiceMethodKind};
use serde_json::Value;
use std::sync::Arc;

pub fn capabilities() -> ServiceCapabilities {
    ServiceCapabilities::from_methods(vec![
        ServiceMethodKind::Custom("read"),
        ServiceMethodKind::Custom("write"),
    ])
}
pub fn register_hooks(
    app: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
) -> anyhow::Result<()> {
    use super::data_hooks::*;
    app.service_hooks("data", |h| {
        h.before(
            ServiceMethodKind::Custom("read"),
            Arc::new(
                dog_auth::hooks::AuthenticateHook::new(auth.clone(), vec!["jwt".into()]).strict(),
            ),
        );
        h.before(ServiceMethodKind::Custom("read"), Arc::new(BeforeRead));
        h.after(ServiceMethodKind::Custom("read"), Arc::new(AfterRead));
        h.before(
            ServiceMethodKind::Custom("write"),
            Arc::new(dog_auth::hooks::AuthenticateHook::new(auth, vec!["jwt".into()]).strict()),
        );
        h.before(ServiceMethodKind::Custom("write"), Arc::new(BeforeWrite));
        h.after(ServiceMethodKind::Custom("write"), Arc::new(AfterWrite));
    });
    Ok(())
}
