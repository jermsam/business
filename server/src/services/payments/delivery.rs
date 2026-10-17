//! At-least-once outbox delivery. Destinations/secrets are server-owned configuration.
use crate::{access, typedb::TypeDBState};
use anyhow::{ensure, Result};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}
pub fn claim_query(event_id: &str, lease: &str, now: i64) -> String {
    format!(
        r#"match $e isa pay_event,has pay_id {},has pay_delivery_status $status,has pay_due $due,has pay_attempts $attempts;
      $due <= {now}; $attempts < 8; {{ $status == "pending"; }} or {{ $status == "delivering"; }};
      (request:$p,event:$e) isa pay_event_owner; (grant:$g,request:$p) isa pay_request_owner;
      $g isa pay_grant,links (merchant:$t,application:$a),has pay_product $product,has pay_environment $environment,has pay_active true;
      $t has biz_revision $revision;
      let $next=$attempts+1; select $t,$e,$p,$product,$environment,$next; distinct;
      update $e has pay_delivery_status "delivering",has pay_due {},has pay_lease {},has pay_attempts == $next;
      fetch {{"id":$e.pay_id,"type":$e.pay_event_type,"operation":$p.pay_id,"reference":$p.pay_reference,
      "merchant":$t.biz_id,"product":$product,"environment":$environment,
      "amount_minor":$p.pay_amount,"currency":$p.pay_currency,"currency_exponent":$p.pay_exponent,"attempt":$next}};"#,
        q(event_id),
        now + 60,
        q(lease)
    )
}
pub fn finish_query(
    event_id: &str,
    lease: &str,
    attempt: i64,
    success: bool,
    now: i64,
) -> Result<String> {
    ensure!((1..=8).contains(&attempt), "Invalid delivery attempt");
    let status = if success {
        "delivered"
    } else if attempt == 8 {
        "dead"
    } else {
        "pending"
    };
    let due = now + 30 * (1_i64 << attempt);
    Ok(format!(
        r#"match $e isa pay_event,has pay_id {},has pay_lease {},has pay_attempts {attempt},has pay_delivery_status "delivering";
      (request:$p,event:$e) isa pay_event_owner; (grant:$g,request:$p) isa pay_request_owner;
      $g isa pay_grant, links (merchant:$t); $t has biz_revision $revision;
      select $t,$e; distinct; update $e has pay_delivery_status "{status}",has pay_due {due},has pay_lease "";
      fetch {{"id":$e.pay_id,"status":$e.pay_delivery_status}};"#,
        q(event_id),
        q(lease)
    ))
}
pub fn signature(secret: &str, event_id: &str, timestamp: i64, body: &[u8]) -> Result<String> {
    ensure!(secret.len() >= 32, "Event signing secret too short");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    mac.update(format!("{timestamp}.{event_id}.").as_bytes());
    mac.update(body);
    Ok(mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub struct Destination {
    pub merchant: String,
    pub product: String,
    pub environment: String,
    pub url: String,
    pub secret: String,
}
/// Consumers acknowledge only after durably storing/deduplicating the event ID.
pub fn verify_signature(
    secret: &str,
    event_id: &str,
    timestamp: i64,
    body: &[u8],
    supplied: &str,
    now: i64,
) -> Result<()> {
    ensure!(now.abs_diff(timestamp) <= 300, "Expired event signature");
    ensure!(
        secret.len() >= 32
            && supplied.len() == 64
            && supplied.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid event signature"
    );
    let bytes: Result<Vec<u8>, _> = (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&supplied[i..i + 2], 16))
        .collect();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    mac.update(format!("{timestamp}.{event_id}.").as_bytes());
    mac.update(body);
    mac.verify_slice(&bytes?)
        .map_err(|_| anyhow::anyhow!("Invalid event signature"))?;
    let payload: Value = serde_json::from_slice(body)?;
    ensure!(payload["id"] == event_id, "Event identity mismatch");
    Ok(())
}
#[allow(dead_code)] // Reserved for a separately configured durable scheduler.
pub fn due_query(destination: &Destination, now: i64) -> String {
    format!(
        r#"match $t isa company,has biz_id {};
      $g isa pay_grant,links (merchant:$t),has pay_product {},has pay_environment {};
      (grant:$g,request:$p) isa pay_request_owner; (request:$p,event:$e) isa pay_event_owner;
      $e has pay_delivery_status $status,has pay_due $due; $due <= {now};
      {{ $status == "pending"; }} or {{ $status == "delivering"; }};
      select $e,$due; distinct; sort $due asc; limit 50;
      fetch {{"id":$e.pay_id}};"#,
        q(&destination.merchant),
        q(&destination.product),
        q(&destination.environment)
    )
}
#[allow(dead_code)] // Used by the scheduler path and crash-recovery tests.
pub fn exhausted_query(event_id: &str, now: i64) -> String {
    format!(
        r#"match $e isa pay_event,has pay_id {},has pay_delivery_status "delivering",has pay_due $due,has pay_attempts 8;
      $due <= {now}; (request:$p,event:$e) isa pay_event_owner; (grant:$g,request:$p) isa pay_request_owner;
      $g isa pay_grant,links (merchant:$t); $t has biz_revision $revision;
      select $t,$e; distinct; update $e has pay_delivery_status "dead",has pay_lease "";
      fetch {{"id":$e.pay_id,"status":$e.pay_delivery_status}};"#,
        q(event_id)
    )
}
/// Explicitly invoked by a scheduler; not enabled by default. Batch size is bounded.
#[allow(dead_code)] // Scheduling is not enabled by deploying the sandbox panel.
pub async fn dispatch_due(state: &TypeDBState, destination: &Destination) -> Result<Value> {
    let now = chrono::Utc::now().timestamp();
    let result = dog_typedb::TypeDBAdapter::new(std::sync::Arc::new(state.clone()))
        .read(serde_json::json!({"query":due_query(destination,now)}))
        .await?;
    let answers = result["ok"]["answers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid outbox response"))?;
    let mut results = Vec::new();
    for answer in answers {
        let id = answer["data"]["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing event identity"))?;
        // Recover final-attempt crashes to a visible dead-letter state.
        if let Ok(mut rows) = access::write_one(state, &exhausted_query(id, now)).await {
            results.push(rows.remove(0));
            continue;
        }
        match deliver(state, id, destination).await {
            Ok(result) => results.push(result),
            Err(_) => results.push(serde_json::json!({"id":id,"status":"unresolved"})),
        }
    }
    Ok(serde_json::json!({"events":results}))
}
/// Caller selects the event from its internal outbox. Never accept arbitrary URLs from clients.
#[allow(dead_code)] // HTTP routes use a shared, bounded connection pool instead.
pub async fn deliver(
    state: &TypeDBState,
    event_id: &str,
    destination: &Destination,
) -> Result<Value> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    deliver_using(state, event_id, destination, http).await
}
// Injection is internal, for the acceptance test's explicitly trusted local CA.
// HTTPS URL validation remains mandatory; production never disables TLS checks.
pub(super) async fn deliver_using(
    state: &TypeDBState,
    event_id: &str,
    destination: &Destination,
    http: reqwest::Client,
) -> Result<Value> {
    let url = reqwest::Url::parse(&destination.url)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none(),
        "Invalid event destination"
    );
    ensure!(
        destination.secret.len() >= 32,
        "Event signing secret too short"
    );
    let lease = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let mut rows = access::write_one(state, &claim_query(event_id, &lease, now)).await?;
    let event = rows.remove(0);
    ensure!(
        event["merchant"] == destination.merchant
            && event["product"] == destination.product
            && event["environment"] == destination.environment,
        "Destination scope mismatch; no event transmitted"
    );
    // Retry bookkeeping is not part of the immutable event payload.
    let mut payload = event.clone();
    payload
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Invalid event"))?
        .remove("attempt");
    let body = serde_json::to_vec(&payload)?;
    let sig = signature(&destination.secret, event_id, now, &body)?;
    let response = http
        .post(url)
        .header("Content-Type", "application/json")
        .header("X-Jitpomi-Event", event_id)
        .header("X-Jitpomi-Timestamp", now.to_string())
        .header("X-Jitpomi-Signature", sig)
        .body(body)
        .send()
        .await;
    let success = response.map(|r| r.status().is_success()).unwrap_or(false);
    let attempt = event["attempt"]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("Invalid attempt"))?;
    let mut result = access::write_one(
        state,
        &finish_query(
            event_id,
            &lease,
            attempt,
            success,
            chrono::Utc::now().timestamp(),
        )?,
    )
    .await?;
    Ok(result.remove(0))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn payload_and_identity_are_signed() {
        let secret = "s".repeat(32);
        let first = signature(&secret, "event-1", 100, b"{}").unwrap();
        assert_ne!(first, signature(&secret, "event-2", 100, b"{}").unwrap());
        assert_ne!(first, signature(&secret, "event-1", 101, b"{}").unwrap());
        assert_ne!(first, signature(&secret, "event-1", 100, b"{ }").unwrap());
    }
    #[test]
    fn receiver_rejects_modified_or_stale_events() {
        let secret = "s".repeat(32);
        let body = br#"{"id":"event-1"}"#;
        let sig = signature(&secret, "event-1", 100, body).unwrap();
        verify_signature(&secret, "event-1", 100, body, &sig, 101).unwrap();
        assert!(verify_signature(&secret, "event-1", 100, body, &sig, 401).is_err());
        assert!(verify_signature(&secret, "event-2", 100, body, &sig, 101).is_err());
        assert!(verify_signature(&secret, "event-1", 100, b"{}", &sig, 101).is_err());
    }
    #[test]
    fn lease_and_attempt_guard_acknowledgement() {
        let query = finish_query("event", "lease", 8, false, 100).unwrap();
        assert!(query.contains("\"dead\""));
        assert!(query.contains("has pay_lease \"lease\""));
        assert!(query.contains("has pay_attempts 8"));
        assert!(finish_query("event", "lease", 0, true, 100).is_err());
    }
}
