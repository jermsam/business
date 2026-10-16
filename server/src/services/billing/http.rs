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
}
pub fn router(engine: Engine) -> Result<Router> {
    let tick_secret = std::env::var("BILLING_TICK_SECRET")?;
    let webhook_secret = std::env::var("STRIPE_WEBHOOK_SECRET").unwrap_or_default();
    ensure!(
        tick_secret.len() >= 32,
        "Billing tick secret must have at least 32 characters"
    );
    Ok(Router::new()
        .route("/internal/billing/tick", post(tick))
        .route("/webhooks/stripe", post(webhook))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(Ingress {
            engine,
            tick_secret,
            webhook_secret,
            slots: Arc::new(tokio::sync::Semaphore::new(1)),
        }))
}
async fn tick(State(state): State<Ingress>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
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
    Ok(Json(result))
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
pub use crate::services::payments::webhook::verify_signature;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_body_timestamp_and_rotating_signatures() {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(b"1000.{\"id\":1}");
        let sig = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let header = format!("t=1000,v1={},v1={sig}", "0".repeat(64));
        assert!(verify_signature("secret", &header, b"{\"id\":1}", 1001).is_ok());
        assert!(verify_signature("secret", &header, b"{ \"id\":1}", 1001).is_err());
        assert!(verify_signature("secret", &header, b"{\"id\":1}", 1301).is_err());
        assert!(verify_signature("secret", &(header + ",t=1000"), b"{\"id\":1}", 1001).is_err());
    }
}
