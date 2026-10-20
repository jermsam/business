pub mod adapters;
#[path = "auth-stripe/mod.rs"]
pub mod auth_stripe;
pub mod charges;
pub mod checkout_sessions;
pub mod customers;
pub mod disputes;
pub mod hooks;
pub mod invoice_items;
pub mod invoice_payments;
pub mod invoices;
pub mod mandates;
pub mod payment_intents;
pub mod payment_methods;
pub mod refunds;
pub mod setup_intents;
use crate::services::BusinessParams;
use dog_core::DogAppBuilder;
use serde_json::Value;
use std::sync::Arc;
pub fn configure(
    builder: &mut DogAppBuilder<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
    state: Arc<adapters::stripe_adapter::StripeState>,
    credentials: Arc<auth_stripe::AuthStripeState>,
) -> anyhow::Result<()> {
    builder.register_service(
        "auth-stripe",
        Arc::new(auth_stripe::AuthStripeService::new(credentials)),
    );
    auth_stripe::auth_stripe_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/customers",
        Arc::new(customers::CustomersService::new(state.clone())),
    );
    customers::customers_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/invoices",
        Arc::new(invoices::InvoicesService::new(state.clone())),
    );
    invoices::invoices_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/invoice-items",
        Arc::new(invoice_items::InvoiceItemsService::new(state.clone())),
    );
    invoice_items::invoice_items_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/checkout-sessions",
        Arc::new(checkout_sessions::CheckoutSessionsService::new(
            state.clone(),
        )),
    );
    checkout_sessions::checkout_sessions_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/payment-methods",
        Arc::new(payment_methods::PaymentMethodsService::new(state.clone())),
    );
    payment_methods::payment_methods_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/setup-intents",
        Arc::new(setup_intents::SetupIntentsService::new(state.clone())),
    );
    setup_intents::setup_intents_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/payment-intents",
        Arc::new(payment_intents::PaymentIntentsService::new(state.clone())),
    );
    payment_intents::payment_intents_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/invoice-payments",
        Arc::new(invoice_payments::InvoicePaymentsService::new(state.clone())),
    );
    invoice_payments::invoice_payments_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/charges",
        Arc::new(charges::ChargesService::new(state.clone())),
    );
    charges::charges_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/refunds",
        Arc::new(refunds::RefundsService::new(state.clone())),
    );
    refunds::refunds_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/disputes",
        Arc::new(disputes::DisputesService::new(state.clone())),
    );
    disputes::disputes_shared::register_hooks(builder, auth.clone())?;
    builder.register_service(
        "stripe/mandates",
        Arc::new(mandates::MandatesService::new(state.clone())),
    );
    mandates::mandates_shared::register_hooks(builder, auth.clone())?;
    Ok(())
}
dog_transport::declare_adapter!(axum, stripe_endpoint, crate::BusinessParams);
pub fn router(http: dog_transport::http::DogHttpService<Value, BusinessParams>) -> axum::Router {
    axum::Router::new()
        .route_service("/stripe/customers", stripe_endpoint(http.clone()))
        .route_service("/stripe/customers/{id}", stripe_endpoint(http.clone()))
        .route_service("/stripe/invoices", stripe_endpoint(http.clone()))
        .route_service("/stripe/invoices/{id}", stripe_endpoint(http.clone()))
        .route_service("/stripe/invoice-items", stripe_endpoint(http.clone()))
        .route_service("/stripe/invoice-items/{id}", stripe_endpoint(http.clone()))
        .route_service("/stripe/checkout-sessions", stripe_endpoint(http.clone()))
        .route_service(
            "/stripe/checkout-sessions/{id}",
            stripe_endpoint(http.clone()),
        )
        .route_service("/stripe/payment-methods", stripe_endpoint(http.clone()))
        .route_service(
            "/stripe/payment-methods/{id}",
            stripe_endpoint(http.clone()),
        )
        .route_service("/stripe/setup-intents", stripe_endpoint(http.clone()))
        .route_service("/stripe/setup-intents/{id}", stripe_endpoint(http.clone()))
        .route_service("/stripe/payment-intents", stripe_endpoint(http.clone()))
        .route_service(
            "/stripe/payment-intents/{id}",
            stripe_endpoint(http.clone()),
        )
        .route_service("/stripe/invoice-payments", stripe_endpoint(http.clone()))
        .route_service(
            "/stripe/invoice-payments/{id}",
            stripe_endpoint(http.clone()),
        )
        .route_service("/stripe/charges", stripe_endpoint(http.clone()))
        .route_service("/stripe/charges/{id}", stripe_endpoint(http.clone()))
        .route_service("/stripe/refunds", stripe_endpoint(http.clone()))
        .route_service("/stripe/refunds/{id}", stripe_endpoint(http.clone()))
        .route_service("/stripe/disputes", stripe_endpoint(http.clone()))
        .route_service("/stripe/disputes/{id}", stripe_endpoint(http.clone()))
        .route_service("/stripe/mandates", stripe_endpoint(http.clone()))
        .route_service("/stripe/mandates/{id}", stripe_endpoint(http.clone()))
        .route_layer(axum::middleware::from_fn(resolve_route))
}
async fn resolve_route(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use dog_core::DogMethod;
    let parts: Vec<_> = request.uri().path().trim_matches('/').split('/').collect();
    let service = format!("stripe/{}", parts[1]);
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
    let capabilities = match parts[1] {
        "customers" => customers::customers_shared::capabilities(),
        "invoices" => invoices::invoices_shared::capabilities(),
        "invoice-items" => invoice_items::invoice_items_shared::capabilities(),
        "checkout-sessions" => checkout_sessions::checkout_sessions_shared::capabilities(),
        "payment-methods" => payment_methods::payment_methods_shared::capabilities(),
        "setup-intents" => setup_intents::setup_intents_shared::capabilities(),
        "payment-intents" => payment_intents::payment_intents_shared::capabilities(),
        "invoice-payments" => invoice_payments::invoice_payments_shared::capabilities(),
        "charges" => charges::charges_shared::capabilities(),
        "refunds" => refunds::refunds_shared::capabilities(),
        "disputes" => disputes::disputes_shared::capabilities(),
        "mandates" => mandates::mandates_shared::capabilities(),
        _ => return axum::http::StatusCode::NOT_FOUND.into_response(),
    };
    let supported = capabilities
        .allowed_methods
        .iter()
        .any(|kind| match (&method, kind) {
            (DogMethod::Find, dog_core::ServiceMethodKind::Find)
            | (DogMethod::Get, dog_core::ServiceMethodKind::Get)
            | (DogMethod::Create, dog_core::ServiceMethodKind::Create)
            | (DogMethod::Patch, dog_core::ServiceMethodKind::Patch)
            | (DogMethod::Remove, dog_core::ServiceMethodKind::Remove) => true,
            (DogMethod::Custom(name), dog_core::ServiceMethodKind::Custom(allowed)) => {
                name == allowed
            }
            _ => false,
        });
    if !supported {
        return axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
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
#[path = "../../../tests/stripe/mod.rs"]
mod tests;
