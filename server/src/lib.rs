use anyhow::Result;
use dog_axum::AxumApp;
use serde_json::Value;

use crate::services::BusinessParams;
use crate::typedb::TypeDBState;

mod app;
mod channels;
mod config;
pub mod hooks;
mod services;
mod typedb;
mod auth;

pub async fn build() -> Result<AxumApp<Value, BusinessParams>> {
    let axum_app = app::axum_app()?;
    TypeDBState::new(axum_app.app.as_ref()).await?;
    let _services = services::configure(axum_app.app.as_ref()).await?;
    let axum_app = axum_app
        .service("/health", || async { "ok" });
    Ok(axum_app)
}