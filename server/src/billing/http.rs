//! Narrow server-to-server ingress; no tenant header controls merchant identity.
use super::{billing_schema::Provider, engine::Engine};
use anyhow::{ensure, Result};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
#[cfg(test)]
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
#[cfg(test)]
use sha2::Sha256;
use std::sync::Arc;
use subtle::ConstantTimeEq;
#[derive(Clone)]
struct Ingress {
    engine: Engine,
    tick_secret: String,
    webhook_secret: String,
    slots: Arc<tokio::sync::Semaphore>,
    account_slots: Arc<tokio::sync::Semaphore>,
}
fn configured(engine: Engine) -> Result<Ingress> {
    let tick_secret = std::env::var("BILLING_TICK_SECRET")?;
    let webhook_secret = std::env::var("STRIPE_WEBHOOK_SECRET").unwrap_or_default();
    ensure!(
        tick_secret.len() >= 32,
        "Billing tick secret must have at least 32 characters"
    );
    Ok(Ingress {
        engine,
        tick_secret,
        webhook_secret,
        slots: Arc::new(tokio::sync::Semaphore::new(1)),
        account_slots: Arc::new(tokio::sync::Semaphore::new(4)),
    })
}
fn routes(state: Ingress) -> Router {
    Router::new()
        .route("/internal/billing/tick", post(tick))
        .route("/webhooks/stripe", post(webhook))
        .route("/onboarding", post(accept_invitation))
        .route("/recovery", post(recover_account))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}
pub fn router(engine: Engine) -> Result<Router> {
    Ok(routes(configured(engine)?))
}
/// One worker and ingress share admission; database claims also protect replicas.
/// The caller owns and aborts the worker on shutdown. No task is started by router().
pub fn runtime(engine: Engine) -> Result<(Router, tokio::task::JoinHandle<()>)> {
    let state = configured(engine)?;
    let router = routes(state.clone());
    let worker = tokio::spawn(async move {
        let mut clock = tokio::time::interval(std::time::Duration::from_secs(60));
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            clock.tick().await;
            let Ok(_slot) = state.slots.try_acquire() else {
                continue;
            };
            match tokio::time::timeout(std::time::Duration::from_secs(300), state.engine.tick())
                .await
            {
                Ok(Ok(report)) => {
                    tracing::info!(merchant=%state.engine.merchant, report=%report, "Billing tick complete");
                }
                Ok(Err(_)) => {
                    tracing::error!(merchant=%state.engine.merchant, "Billing tick failed; inspect durable claims before retrying uncertain effects");
                }
                Err(_) => {
                    tracing::error!(merchant=%state.engine.merchant, "Billing tick deadline exceeded; durable claims preserved");
                }
            }
        }
    });
    Ok((router, worker))
}

async fn tick(
    State(state): State<Ingress>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let received = headers
        .get("authorization")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    if !bool::from(state.tick_secret.as_bytes().ct_eq(received.as_bytes())) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let _slot = state
        .slots
        .try_acquire()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    // Cancellation preserves durable claims. No dropped worker can replay a charge.
    let result = tokio::time::timeout(std::time::Duration::from_secs(50), state.engine.tick())
        .await
        .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let status = if result["errors"].as_u64().unwrap_or(1) == 0 {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    Ok((status, Json(result)))
}
async fn webhook(
    State(state): State<Ingress>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, StatusCode> {
    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    verify_signature(
        &state.webhook_secret,
        signature,
        &body,
        chrono::Utc::now().timestamp(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let event: Value = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    if event["account"].as_str() != state.engine.providers.connected_account.as_deref() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if event["livemode"].as_bool() != Some(state.engine.providers.live) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !event["type"].as_str().unwrap_or("").starts_with("invoice.") {
        return Ok(Json(json!({"received":true})));
    }
    let Some(id) = event
        .pointer("/data/object/metadata/business_invoice")
        .and_then(Value::as_str)
    else {
        return Ok(Json(json!({"received":true})));
    };
    let _slot = state
        .slots
        .try_acquire()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let external = event
        .pointer("/data/object/id")
        .and_then(Value::as_str)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let result = async {
        let observation = state
            .engine
            .providers
            .observe(Provider::Stripe, external)
            .await?;
        state
            .engine
            .apply_observation(id, Provider::Stripe, &observation)
            .await
    };
    tokio::time::timeout(std::time::Duration::from_secs(45), result)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    // ACK only after the database write; duplicates reconcile current state.
    Ok(Json(json!({"received":true})))
}
// Preserve existing callers while exposing verification to other payment consumers.
pub use super::signature::verify_signature;
#[cfg(test)]
#[path = "../../tests/billing/http.rs"]
mod tests;

async fn account_action(
    state: Ingress,
    headers: HeaderMap,
    data: Value,
    kind: &str,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let secret = std::env::var("BUSINESS_GATEWAY_SECRET")
        .or_else(|_| std::env::var("PORTAL_ORIGIN_SECRET"))
        .unwrap_or_default();
    let received = headers
        .get("x-gateway-secret")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("");
    if secret.len() < 32 || !bool::from(secret.as_bytes().ct_eq(received.as_bytes())) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"message":"Please use the account website."})),
        ));
    }
    // Bound expensive password work independently of the billing worker.
    let _slot = state.account_slots.try_acquire().map_err(|_| {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"message":"Please wait a moment and try again."})),
        )
    })?;
    // Gateway has per-IP limits; the DB additionally limits reset mail per account.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(40),
        state.engine.public_account_action(kind, &data),
    )
    .await;
    match result {
        Ok(Ok(value)) => Ok(Json(value)),
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"message":"We couldn’t use that link. Check your details or request a new link."}),
            ),
        )),
    }
}
async fn accept_invitation(
    State(state): State<Ingress>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if data["action"] != "accept" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"message":"Action unavailable"})),
        ));
    }
    account_action(state, headers, data, "invite").await
}
async fn recover_account(
    State(state): State<Ingress>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if data["action"] != "request" && data["action"] != "reset" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"message":"Action unavailable"})),
        ));
    }
    account_action(state, headers, data, "reset").await
}
