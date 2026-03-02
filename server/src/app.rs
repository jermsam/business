use crate::channels::channels;
use crate::config::config;
use crate::hooks::global_hook;
use crate::services::BusinessParams;
use crate::auth;
use anyhow::Result;
use dog_axum::{axum, AxumApp};
use dog_core::DogApp;
use serde_json::Value;

pub fn axum_app () -> Result<AxumApp<Value,BusinessParams>> {
    dotenvy::dotenv()?;
    let app:DogApp<Value, BusinessParams> = DogApp::new();
    config(&app)?;
    global_hook(&app)?;  // Register global hooks BEFORE services
    auth::strategies(&app)?;
    channels(&app)?;

    let axum = axum(app);
    Ok(axum)
}