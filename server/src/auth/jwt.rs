use dog_auth::{AuthenticationService, JwtStrategy};
use std::sync::Arc;

pub fn register_jwt<P: Send + Clone + 'static>(auth: &AuthenticationService<P>) {
    auth.register_strategy("jwt", Arc::new(JwtStrategy::new(&auth.base)));
}
