use dog_core::{ServiceCapabilities, ServiceMethodKind::*};
pub fn capabilities() -> ServiceCapabilities {
    ServiceCapabilities::from_methods(vec![Find, Get, Create, Update, Patch, Remove])
}
