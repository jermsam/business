use std::sync::Arc;

use crate::services::BusinessParams;
pub fn register_hooks(
    app: &mut dog_core::DogAppBuilder<serde_json::Value, BusinessParams>,
) -> anyhow::Result<()> {
    app.service_hooks("authentication", |h| {
        h.before_create(Arc::new(super::authentication_hooks::LogAuthCreate));
        h.after_create(Arc::new(
            super::authentication_hooks::StripPasswordFromAuthResult,
        ));
        h.before_remove(Arc::new(super::authentication_hooks::LogAuthRemove));
    });
    Ok(())
}
