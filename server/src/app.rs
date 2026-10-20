use crate::BusinessParams;
use anyhow::Result;
use dog_core::DogApp;
use dog_transport::{http::DogHttpService, IntoDogService};
use serde_json::Value;
use std::sync::Arc;
pub type Backend = (
    DogApp<Value, BusinessParams>,
    DogHttpService<Value, BusinessParams>,
);
/// Startup only connects and verifies; provisioning is an explicit operator action.
pub async fn build() -> Result<Backend> {
    if let Err(error) = match std::env::var("BUSINESS_ENV_FILE") {
        Ok(path) => dotenvy::from_path(path),
        Err(_) => dotenvy::dotenv().map(|_| ()),
    } {
        if !error.not_found() {
            return Err(error.into());
        }
    }
    let mut builder = dog_core::DogAppBuilder::new();
    crate::config::config(&mut builder)?;
    crate::typedb::TypeDBState::initialize(&mut builder).await?;
    compose(builder)
}
/// The same composition used by the executable, tools and integration tests.
pub fn build_with_state(
    state: Arc<crate::typedb::TypeDBState>,
    jwt_secret: String,
) -> Result<Backend> {
    anyhow::ensure!(
        jwt_secret.trim().len() >= 32,
        "JWT secret must contain at least 32 nonblank bytes"
    );
    let mut builder = dog_core::DogAppBuilder::new();
    builder.set("auth.jwt.secret", jwt_secret);
    builder.set("typedb", state);
    compose(builder)
}
fn compose(mut builder: dog_core::DogAppBuilder<Value, BusinessParams>) -> Result<Backend> {
    let state: Arc<crate::typedb::TypeDBState> = builder
        .get("typedb")
        .ok_or_else(|| anyhow::anyhow!("TypeDB state missing"))?;
    let auth = crate::auth::strategies(&mut builder, state.clone())?;
    crate::hooks::global_hook(&mut builder)?;
    crate::channels::channels(&mut builder)?;
    crate::services::configure(&mut builder, auth.clone(), state)?;
    let app = builder.build();
    auth.setup(app.clone());
    let http = app.clone().into_service(
        dog_transport::HttpOptions::default()
            .tenant_header("x-tenant-id")
            .body_limit(65536),
    );
    Ok((app, http))
}
dog_transport::declare_adapter!(axum, endpoint, crate::BusinessParams);
pub fn router(http: DogHttpService<Value, BusinessParams>) -> axum::Router {
    axum::Router::new()
        .route_service("/authentication", endpoint(http.clone()))
        .route_service("/data", endpoint(http.clone()))
        .merge(crate::mercury::router(http.clone()))
        .merge(crate::stripe::router(http))
        .route_layer(axum::middleware::from_fn(crate::admission::guard))
        .route(
            "/queries/data/{name}",
            axum::routing::get(
                |axum::extract::Path(name): axum::extract::Path<String>| async move {
                    let name = name.strip_suffix(".tql").unwrap_or(&name);
                    match crate::services::data::query_assets::ASSETS
                        .iter()
                        .find(|(asset, _)| *asset == name)
                    {
                        Some((_, source)) => (
                            axum::http::StatusCode::OK,
                            [(
                                axum::http::header::CONTENT_TYPE,
                                "text/plain; charset=utf-8",
                            )],
                            *source,
                        ),
                        None => (
                            axum::http::StatusCode::NOT_FOUND,
                            [(
                                axum::http::header::CONTENT_TYPE,
                                "text/plain; charset=utf-8",
                            )],
                            "",
                        ),
                    }
                },
            ),
        )
        .route(
            "/data-client.js",
            axum::routing::get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/javascript")],
                    include_str!("../public/data-client.js"),
                )
            }),
        )
        .route("/health", axum::routing::get(|| async { "ok" }))
}
