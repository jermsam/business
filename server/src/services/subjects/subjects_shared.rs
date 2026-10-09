use crate::services::BusinessParams;
use dog_core::{DogAppBuilder, ServiceCapabilities, ServiceMethodKind};
pub fn capabilities() -> ServiceCapabilities {
    ServiceCapabilities::from_methods(vec![
        ServiceMethodKind::Custom("read"),
        ServiceMethodKind::Custom("write"),
    ])
}
pub fn register_hooks(
    _builder: &mut DogAppBuilder<serde_json::Value, BusinessParams>,
) -> anyhow::Result<()> {
    Ok(())
}
