use dog_core::{ServiceCapabilities, ServiceMethodKind::*};
#[derive(Clone, Copy)]
pub enum Kind {
    Customers,
    Plans,
    Invoices,
}
pub fn capabilities(kind: Kind) -> ServiceCapabilities {
    ServiceCapabilities::from_methods(match kind {
        Kind::Invoices => vec![Find, Get],
        _ => vec![Find, Get, Create, Patch],
    })
}
