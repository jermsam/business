use crate::services::types::BusinessParams;
use dog_core::{ServiceCapabilities, ServiceMethodKind};
use std::sync::Arc;

pub fn capabilities() -> ServiceCapabilities {
    ServiceCapabilities::from_methods(vec![
        ServiceMethodKind::Custom("read"),
        ServiceMethodKind::Custom("write"),
    ])
}

pub fn register_hooks(
    app: &dog_core::DogApp<serde_json::Value, BusinessParams>,
) -> anyhow::Result<()> {
    app.service("subjects")?.hooks(|h| {
        h.before_find(Arc::new(super::subjects_hooks::BeforeRead));
        h.after_find(Arc::new(super::subjects_hooks::AfterRead));
        h.before_create(Arc::new(super::subjects_hooks::BeforeWrite));
        h.after_create(Arc::new(super::subjects_hooks::AfterWrite));
    });

    Ok(())
}
