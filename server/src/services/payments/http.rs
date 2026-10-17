//! Opt-in sandbox-only service endpoints. Never authenticate product identity from a body.
use super::{
    checkout,
    operation::{Environment, OperationScope},
    repository,
    requests::PaymentRequest,
    stripe::StripeTransport,
};
use crate::typedb::TypeDBState;
use anyhow::{ensure, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Application {
    pub id: String,
    pub token_sha256: String,
    pub merchant: String,
    pub product: String,
    pub success_url: String,
    pub cancel_url: String,
    #[serde(default)]
    pub delivery: Option<DeliveryConfig>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryConfig {
    pub url: String,
    pub secret: String,
}
#[derive(Clone)]
struct Api {
    state: Arc<TypeDBState>,
    applications: Vec<Application>,
    transport: StripeTransport,
    delivery_http: reqwest::Client,
    slots: Arc<tokio::sync::Semaphore>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub operation_id: String,
    pub reference: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: u8,
    #[serde(default)]
    pub session_id: Option<String>,
}
impl Input {
    fn request(&self, app: &Application) -> PaymentRequest {
        PaymentRequest {
            scope: OperationScope {
                merchant_id: app.merchant.clone(),
                product_id: app.product.clone(),
                environment: Environment::Sandbox,
                operation_id: self.operation_id.clone(),
            },
            reference: self.reference.clone(),
            amount_minor: self.amount_minor,
            currency: self.currency.clone(),
            currency_exponent: self.currency_exponent,
        }
    }
}
fn identity<'a>(
    headers: &HeaderMap,
    apps: &'a [Application],
) -> Result<&'a Application, StatusCode> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !(32..=512).contains(&token.len()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let hash = format!("{:x}", Sha256::digest(token.as_bytes()));
    apps.iter()
        .find(|app| bool::from(hash.as_bytes().ct_eq(app.token_sha256.as_bytes())))
        .ok_or(StatusCode::UNAUTHORIZED)
}
pub fn router(state: Arc<TypeDBState>, applications: Vec<Application>) -> Result<Router> {
    build_router(state, applications, None)
}
#[cfg(test)]
pub(super) fn router_with_delivery_ca(
    state: Arc<TypeDBState>,
    applications: Vec<Application>,
    ca: reqwest::Certificate,
) -> Result<Router> {
    build_router(state, applications, Some(ca))
}
fn build_router(
    state: Arc<TypeDBState>,
    applications: Vec<Application>,
    ca: Option<reqwest::Certificate>,
) -> Result<Router> {
    ensure!(
        state.database == "business_dev"
            || (cfg!(test) && state.database.starts_with("payments_test_")),
        "Shared payment endpoints are sandbox-only"
    );
    ensure!(
        !applications.is_empty() && applications.len() <= 32,
        "Configure payment applications"
    );
    let mut ids = std::collections::HashSet::new();
    let mut hashes = std::collections::HashSet::new();
    for app in &applications {
        ensure!(
            ids.insert(app.id.clone()) && hashes.insert(app.token_sha256.clone()),
            "Duplicate payment application or credential"
        );
        ensure!(
            app.token_sha256.len() == 64
                && app
                    .token_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "Invalid credential digest"
        );
        let request = Input {
            operation_id: "validation".into(),
            reference: "validation".into(),
            amount_minor: 100,
            currency: "USD".into(),
            currency_exponent: 2,
            session_id: None,
        }
        .request(app);
        checkout::form(&request, &app.success_url, &app.cancel_url)?;
        if let Some(config) = &app.delivery {
            let url = reqwest::Url::parse(&config.url)?;
            ensure!(
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && config.secret.len() >= 32,
                "Invalid event delivery configuration"
            );
        }
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(std::time::Duration::from_secs(10));
    if let Some(ca) = ca {
        builder = builder.add_root_certificate(ca);
    }
    let client = builder.build()?;
    Ok(Router::new()
        .route("/payments/v1/checkouts", post(start))
        .route("/payments/v1/status", post(status))
        .route("/payments/v1/reconcile", post(reconcile))
        .route("/payments/v1/events/dispatch", post(dispatch))
        .route("/payments/sandbox/receiver", post(receive_sandbox))
        .layer(DefaultBodyLimit::max(8192))
        .with_state(Api {
            state,
            applications,
            transport: StripeTransport::new(client.clone(), false),
            delivery_http: client,
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
        }))
}
// Demonstration consumer only: records a verified receipt, never fulfils an order.
async fn receive_sandbox(
    State(api): State<Api>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, StatusCode> {
    let field = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)
    };
    let id = field("x-jitpomi-event")?;
    let timestamp = field("x-jitpomi-timestamp")?
        .parse::<i64>()
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let payload: Value = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let app = api.applications.iter().find(|app| payload["merchant"] == app.merchant && payload["product"] == app.product
        && app.delivery.as_ref().is_some_and(|d| d.url == "https://jitpomi-business-validation.onrender.com/payments/sandbox/receiver"))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let config = app.delivery.as_ref().ok_or(StatusCode::UNAUTHORIZED)?;
    super::delivery::verify_signature(
        &config.secret,
        id,
        timestamp,
        &body,
        field("x-jitpomi-signature")?,
        chrono::Utc::now().timestamp(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if payload["environment"] != "sandbox" || payload["type"] != "payment.paid" {
        return Err(StatusCode::BAD_REQUEST);
    }
    let q = |value: &Value| serde_json::to_string(value).map_err(|_| StatusCode::BAD_REQUEST);
    let hash = format!("{:x}", Sha256::digest(&body));
    let read = api
        .state
        .driver
        .transaction(&api.state.database, typedb_driver::TransactionType::Read)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let existing = crate::access::documents(
        &read,
        &format!(
            "match $r isa pay_sandbox_receipt,has pay_id {};fetch {{\"hash\":$r.pay_body_hash}};",
            q(&json!(id))?
        ),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let _ = read.close().await;
    if let Some(receipt) = existing.first() {
        if receipt["hash"] != hash {
            return Err(StatusCode::CONFLICT);
        }
        return Ok(Json(json!({"received":true,"duplicate":true})));
    }
    let query = format!(
        r#"match $t isa company,has biz_id {};
      $a isa pay_application,has pay_id {}; $g isa pay_grant,links (merchant:$t,application:$a),has pay_product {},has pay_environment "sandbox",has pay_active true;
      (grant:$g,request:$p) isa pay_request_owner; $p has pay_id {},has pay_status "paid",has pay_reference {},has pay_amount {},has pay_currency {},has pay_exponent {};
      (request:$p,event:$e) isa pay_event_owner; $e has pay_id {},has pay_event_type "payment.paid";
      not {{$r isa pay_sandbox_receipt,has pay_id {};}};
      insert $r isa pay_sandbox_receipt,has pay_id {},has pay_body_hash "{hash}",has pay_created {};
      fetch {{"id":$r.pay_id}};"#,
        q(&payload["merchant"])?,
        q(&json!(app.id))?,
        q(&payload["product"])?,
        q(&payload["operation"])?,
        q(&payload["reference"])?,
        q(&payload["amount_minor"])?,
        q(&payload["currency"])?,
        q(&payload["currency_exponent"])?,
        q(&json!(id))?,
        q(&json!(id))?,
        q(&json!(id))?,
        chrono::Utc::now().timestamp()
    );
    crate::access::write_one(&api.state, &query)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(json!({"received":true,"duplicate":false})))
}
async fn dispatch(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<Value>, StatusCode> {
    let app = identity(&headers, &api.applications)?;
    let _slot = api
        .slots
        .try_acquire()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    let request = input.request(app);
    request.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let stored = repository::lookup(&api.state, &app.id, &request)
        .await
        .map_err(|_| StatusCode::CONFLICT)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if stored["status"] != "paid" {
        return Err(StatusCode::CONFLICT);
    }
    let config = app
        .delivery
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let destination = super::delivery::Destination {
        merchant: app.merchant.clone(),
        product: app.product.clone(),
        environment: "sandbox".into(),
        url: config.url.clone(),
        secret: config.secret.clone(),
    };
    let event = format!(
        "{}-paid",
        request
            .scope
            .idempotency_key()
            .map_err(|_| StatusCode::BAD_REQUEST)?
    );
    let response=dog_typedb::TypeDBAdapter::new(api.state.clone()).read(json!({"query":format!("match $e isa pay_event,has pay_id {};fetch {{\"status\":$e.pay_delivery_status}};",serde_json::to_string(&event).map_err(|_|StatusCode::BAD_REQUEST)?)})).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    let rows = response["ok"]["answers"]
        .as_array()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if rows.len() == 1 && rows[0]["data"]["status"] == "delivered" {
        return Ok(Json(json!({"id":event,"status":"delivered"})));
    }
    super::delivery::deliver_using(&api.state, &event, &destination, api.delivery_http.clone())
        .await
        .map(Json)
        .map_err(|_| StatusCode::CONFLICT)
}
async fn start(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<Value>, StatusCode> {
    let app = identity(&headers, &api.applications)?;
    let _slot = api
        .slots
        .try_acquire()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    let request = input.request(app);
    request.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    // Authenticate and authorize before creating a request or calling Stripe.
    checkout::start(
        &api.state,
        &app.id,
        &request,
        &api.transport,
        &app.success_url,
        &app.cancel_url,
    )
    .await
    .map(Json)
    .map_err(|_| StatusCode::CONFLICT)
}
async fn status(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<Value>, StatusCode> {
    let app = identity(&headers, &api.applications)?;
    let _slot = api
        .slots
        .try_acquire()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    let request = input.request(app);
    request.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let result = repository::lookup(&api.state, &app.id, &request)
        .await
        .map_err(|_| StatusCode::CONFLICT)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({"id":result["id"],"status":result["status"]})))
}
async fn reconcile(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<Value>, StatusCode> {
    let app = identity(&headers, &api.applications)?;
    let _slot = api
        .slots
        .try_acquire()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    let request = input.request(app);
    request.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let session = input.session_id.as_deref().ok_or(StatusCode::BAD_REQUEST)?;
    checkout::reconcile(&api.state, &app.id, &request, &api.transport, session)
        .await
        .map(Json)
        .map_err(|_| StatusCode::CONFLICT)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bearer_identity_cannot_be_selected_by_a_tenant_header() {
        let token = "a".repeat(40);
        let app = Application {
            id: "test".into(),
            token_sha256: format!("{:x}", Sha256::digest(token.as_bytes())),
            merchant: "merchant".into(),
            product: "product".into(),
            success_url: "https://example.com/ok".into(),
            cancel_url: "https://example.com/cancel".into(),
            delivery: None,
        };
        let apps = vec![app];
        let mut headers = HeaderMap::new();
        assert!(identity(&headers, &apps).is_err());
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        headers.insert("x-tenant-id", "other".parse().unwrap());
        assert_eq!(identity(&headers, &apps).unwrap().merchant, "merchant");
        headers.insert("authorization", "Bearer invalid".parse().unwrap());
        assert!(identity(&headers, &apps).is_err());
    }
}
