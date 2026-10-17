use dog_core::{ServiceCapabilities, ServiceMethodKind};
pub fn capabilities() -> ServiceCapabilities {
    ServiceCapabilities::from_methods(vec![ServiceMethodKind::Patch])
}
