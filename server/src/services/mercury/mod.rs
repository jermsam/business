pub mod accounts;
pub mod adapters;
#[path = "auth-mercury/mod.rs"]
pub mod auth_mercury;
pub mod customers;
pub mod hooks;
pub mod invoices;
pub mod transactions;
use crate::services::BusinessParams;
use adapters::mercury_adapter::MercuryState;
use dog_core::DogAppBuilder;
use serde_json::Value;
use std::sync::Arc;

/// Mercury can be composed without TypeDB. Credentials are tenant-bound and
/// strict dog-auth hooks run for every provider call, including internal calls.
pub fn configure(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
    state: Arc<MercuryState>,
    credentials: Arc<auth_mercury::AuthMercuryState>,
) -> anyhow::Result<()> {
    builder.register_service(
        "auth-mercury",
        Arc::new(auth_mercury::AuthMercuryService::new(credentials)),
    );
    auth_mercury::auth_mercury_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "mercury/accounts",
        Arc::new(accounts::AccountsService::new(state.clone())),
    );
    builder.register_service(
        "mercury/customers",
        Arc::new(customers::CustomersService::new(state.clone())),
    );
    builder.register_service(
        "mercury/invoices",
        Arc::new(invoices::InvoicesService::new(state.clone())),
    );
    builder.register_service(
        "mercury/transactions",
        Arc::new(transactions::TransactionsService::new(state.clone())),
    );
    accounts::accounts_shared::register_hooks(builder, auth.clone())?;
    customers::customers_shared::register_hooks(builder, auth.clone())?;
    invoices::invoices_shared::register_hooks(builder, auth.clone())?;
    transactions::transactions_shared::register_hooks(builder, auth)?;
    Ok(())
}
dog_transport::declare_adapter!(axum, mercury_endpoint, crate::BusinessParams);
pub fn router(http: dog_transport::http::DogHttpService<Value, BusinessParams>) -> axum::Router {
    axum::Router::new()
        .route_service("/mercury/accounts", mercury_endpoint(http.clone()))
        .route_service("/mercury/accounts/{id}", mercury_endpoint(http.clone()))
        .route_service("/mercury/customers", mercury_endpoint(http.clone()))
        .route_service("/mercury/customers/{id}", mercury_endpoint(http.clone()))
        .route_service("/mercury/invoices", mercury_endpoint(http.clone()))
        .route_service("/mercury/invoices/{id}", mercury_endpoint(http.clone()))
        .route_service("/mercury/transactions", mercury_endpoint(http.clone()))
        .route_service("/mercury/transactions/{id}", mercury_endpoint(http))
        .route_layer(axum::middleware::from_fn(resolve_route))
}
async fn resolve_route(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use dog_core::DogMethod;
    let parts: Vec<_> = request.uri().path().trim_matches('/').split('/').collect();
    let service = format!("mercury/{}", parts[1]);
    let id = parts.get(2).map(|id| id.to_string());
    let method = if let Some(custom) = request
        .headers()
        .get("x-service-method")
        .and_then(|h| h.to_str().ok())
    {
        if request.method() != axum::http::Method::POST || id.is_some() {
            return axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        DogMethod::Custom(custom.into())
    } else {
        match *request.method() {
            axum::http::Method::GET => {
                if id.is_some() {
                    DogMethod::Get
                } else {
                    DogMethod::Find
                }
            }
            axum::http::Method::POST => DogMethod::Create,
            axum::http::Method::PUT => DogMethod::Update,
            axum::http::Method::PATCH => DogMethod::Patch,
            axum::http::Method::DELETE => DogMethod::Remove,
            _ => return axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response(),
        }
    };
    request
        .extensions_mut()
        .insert(dog_transport::http::HttpRoute {
            service,
            method,
            id,
        });
    next.run(request).await
}
use axum::response::IntoResponse;
#[cfg(test)]
#[path = "../../../tests/mercury/mod.rs"]
mod tests;
