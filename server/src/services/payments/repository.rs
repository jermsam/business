//! Internal query boundary. No routes or provider side effects; migration 009 required.
//! Application identity MUST come from a verified server credential, not a caller body.
use super::{operation::Environment, requests::PaymentRequest};
use crate::{access, typedb::TypeDBState};
use anyhow::{ensure, Result};
use serde_json::Value;

fn q(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization")
}
fn authorization(application: &str, request: &PaymentRequest) -> Result<String> {
    request.validate()?;
    ensure!(
        !application.is_empty() && application.len() <= 128,
        "Invalid application identity"
    );
    let environment = match request.scope.environment {
        Environment::Sandbox => "sandbox",
        Environment::Live => "live",
    };
    Ok(format!(
        r#"match $a isa pay_application, has pay_id {}; $t isa company, has biz_id {}, has biz_revision $revision;
      $g isa pay_grant; let $allowed=pay_authorized($a,$t,$g,{},{}); $allowed==true;"#,
        q(application),
        q(&request.scope.merchant_id),
        q(&request.scope.product_id),
        q(environment)
    ))
}
pub(super) fn request_match(
    application: &str,
    request: &PaymentRequest,
    status: &str,
) -> Result<String> {
    Ok(format!(
        r#"{} $p isa pay_request, has pay_id {}, has pay_fingerprint {}, has pay_status {};
      (grant:$g,request:$p) isa pay_request_owner;"#,
        authorization(application, request)?,
        q(&request.scope.idempotency_key()?),
        q(&request.fingerprint()?),
        q(status)
    ))
}
pub fn create_query(application: &str, request: &PaymentRequest, now: i64) -> Result<String> {
    let auth = authorization(application, request)?;
    let key = request.scope.idempotency_key()?;
    // @key prevents duplicate inserts. The transaction updates $t to conflict with
    // merchant authorization changes using the established guarded-write boundary.
    Ok(format!(
        r#"{auth}
      not {{ $existing isa pay_request, has pay_id {}; }};
      select $t,$g; distinct;
      insert $p isa pay_request, has pay_id {}, has pay_fingerprint {}, has pay_amount {},
       has pay_currency {}, has pay_exponent {}, has pay_reference {}, has pay_status "requested", has pay_created {now};
      (grant:$g,request:$p) isa pay_request_owner;
      $e isa pay_event, has pay_id {}, has pay_event_type "payment.requested", has pay_created {now},
       has pay_delivery_status "pending", has pay_attempts 0, has pay_due {now}, has pay_lease "";
      (request:$p,event:$e) isa pay_event_owner;
      fetch {{"id":$p.pay_id,"fingerprint":$p.pay_fingerprint,"status":$p.pay_status}};"#,
        q(&key),
        q(&key),
        q(&request.fingerprint()?),
        request.amount_minor,
        q(&request.currency),
        request.currency_exponent,
        q(&request.reference),
        q(&format!("{key}-requested"))
    ))
}
pub fn lookup_query(application: &str, request: &PaymentRequest) -> Result<String> {
    Ok(format!(
        r#"{} $p isa pay_request, has pay_id {}; (grant:$g,request:$p) isa pay_request_owner;
      fetch {{"id":$p.pay_id,"fingerprint":$p.pay_fingerprint,"status":$p.pay_status}};"#,
        authorization(application, request)?,
        q(&request.scope.idempotency_key()?)
    ))
}
pub fn claim_query(application: &str, request: &PaymentRequest) -> Result<String> {
    Ok(format!(
        r#"{} $p isa pay_request, has pay_id {}, has pay_fingerprint {}, has pay_status "requested";
      (grant:$g,request:$p) isa pay_request_owner; select $t,$p; distinct;
      update $p has pay_status "processing";
      fetch {{"id":$p.pay_id,"fingerprint":$p.pay_fingerprint,"status":$p.pay_status}};"#,
        authorization(application, request)?,
        q(&request.scope.idempotency_key()?),
        q(&request.fingerprint()?)
    ))
}
/// The caller resolves/reconciles any uncertain commit; never charge on an error.
pub async fn create(
    state: &TypeDBState,
    application: &str,
    request: &PaymentRequest,
    now: i64,
) -> Result<Value> {
    let mut rows = access::write_one(state, &create_query(application, request, now)?).await?;
    Ok(rows.remove(0))
}
pub async fn claim(
    state: &TypeDBState,
    application: &str,
    request: &PaymentRequest,
) -> Result<Value> {
    let mut rows = access::write_one(state, &claim_query(application, request)?).await?;
    Ok(rows.remove(0))
}
pub fn verify_replay(request: &PaymentRequest, stored: &Value) -> Result<()> {
    ensure!(
        stored["id"].as_str() == Some(request.scope.idempotency_key()?.as_str()),
        "Payment identity mismatch"
    );
    ensure!(
        stored["fingerprint"].as_str() == Some(request.fingerprint()?.as_str()),
        "Payment operation payload conflict"
    );
    Ok(())
}
/// Replays still query the active grant; a known operation does not bypass revocation.
pub async fn lookup(
    state: &TypeDBState,
    application: &str,
    request: &PaymentRequest,
) -> Result<Option<Value>> {
    let result = dog_typedb::TypeDBAdapter::new(std::sync::Arc::new(state.clone()))
        .read(serde_json::json!({"query":lookup_query(application,request)?}))
        .await?;
    let rows = result["ok"]["answers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid payment query response"))?;
    ensure!(rows.len() <= 1, "Ambiguous payment authorization");
    let stored = rows.first().map(|row| row["data"].clone());
    if let Some(ref stored) = stored {
        verify_replay(request, stored)?;
    }
    Ok(stored)
}
/// A concurrent creation may return a conflict/uncertain result. The consumer can
/// call again with the same scope, but MUST NOT execute provider I/O on that error.
pub async fn create_or_get(
    state: &TypeDBState,
    application: &str,
    request: &PaymentRequest,
    now: i64,
) -> Result<Value> {
    if let Some(stored) = lookup(state, application, request).await? {
        return Ok(stored);
    }
    create(state, application, request, now).await
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PaymentRequest {
        serde_json::from_value(serde_json::json!({"scope":{"merchant_id":"jitpomi","product_id":"shop","environment":"sandbox","operation_id":"order-1"},"amount_minor":100,"currency":"USD","currency_exponent":2,"reference":"cart-1"})).unwrap()
    }
    #[test]
    fn queries_keep_authorization_and_atomic_event() {
        let request = fixture();
        let query = create_query("shop-service", &request, 100).unwrap();
        assert!(query.contains("pay_authorized"));
        assert!(query.contains("pay_event_owner"));
        assert!(query.contains("select $t,$g"));
        assert!(query.contains("not { $existing"));
        assert!(lookup_query("shop-service", &request)
            .unwrap()
            .contains("grant:$g,request:$p"));
        let claim = claim_query("shop-service", &request).unwrap();
        assert!(claim.contains("has pay_status \"requested\""));
        assert!(claim.contains("has pay_fingerprint"));
        assert!(claim.contains("select $t,$p"));
    }
    #[test]
    fn changed_payload_is_not_a_successful_replay() {
        let request = fixture();
        let stored = serde_json::json!({"id":request.scope.idempotency_key().unwrap(),"fingerprint":request.fingerprint().unwrap()});
        verify_replay(&request, &stored).unwrap();
        let mut changed = request;
        changed.amount_minor = 101;
        assert!(verify_replay(&changed, &stored).is_err());
    }
}
