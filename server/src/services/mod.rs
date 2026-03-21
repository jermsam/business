use dog_auth::AuthenticationService;
use dog_core::{DogApp, DogService};

use crate::typedb::TypeDBState;

use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

mod types;
pub use types::*;

mod authentication;
mod subjects;

pub struct BusinessService {
    pub auth_svc: Arc<dyn DogService<serde_json::Value, BusinessParams>>,
    pub subjects: Arc<dyn DogService<serde_json::Value, BusinessParams>>,
}

pub async fn configure(app: &DogApp<Value, BusinessParams>) -> Result<BusinessService> {
    let _state: Arc<TypeDBState> = app
        .get("typedb")
        .expect("TypeDBState should be initialised before configure() is called");

    // Register authentication service
    let auth_core = AuthenticationService::from_app(app)
        .ok_or_else(|| anyhow::anyhow!("AuthenticationService missing from app state; did you call crate::auth::strategies(&dog_app) during startup?"))?;
    let auth_svc: Arc<dyn DogService<Value, BusinessParams>> =
        Arc::new(authentication::AuthService::new(auth_core));
    app.register_service("authentication", Arc::clone(&auth_svc));
    authentication::authentication_shared::register_hooks(app)?;

    let subjects: Arc<dyn DogService<serde_json::Value, BusinessParams>> =
        Arc::new(subjects::SubjectsService::new(Arc::clone(&_state)));
    app.register_service("subjects", Arc::clone(&subjects));
    subjects::subjects_shared::register_hooks(app)?;

    Ok(BusinessService { auth_svc, subjects })
}
