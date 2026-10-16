//! One-time hosted checkout. Internal only: authenticated service admission is required.
use super::{
    operation::Environment, repository, requests::PaymentRequest, stripe::StripeTransport,
};
use crate::{access, typedb::TypeDBState};
use anyhow::{ensure, Result};
use reqwest::Method;
use serde_json::Value;
fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

pub fn form(
    request: &PaymentRequest,
    success: &str,
    cancel: &str,
) -> Result<Vec<(String, String)>> {
    request.validate()?;
    // Deliberately small tested currency set; never infer exponent from a user field.
    let exponent = match request.currency.as_str() {
        "USD" | "EUR" | "GBP" | "KES" => 2,
        "UGX" | "RWF" | "JPY" => 0,
        _ => anyhow::bail!("Currency not enabled for checkout"),
    };
    ensure!(
        request.currency_exponent == exponent,
        "Currency exponent mismatch"
    );
    for target in [success, cancel] {
        let url = reqwest::Url::parse(target)?;
        ensure!(
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "Invalid configured return URL"
        );
    }
    // URLs must be selected from the authenticated product's server-owned configuration.
    let key = request.scope.idempotency_key()?;
    Ok(vec![
        ("mode".into(), "payment".into()),
        ("success_url".into(), success.into()),
        ("cancel_url".into(), cancel.into()),
        ("client_reference_id".into(), key.clone()),
        ("metadata[payment_operation]".into(), key),
        (
            "line_items[0][price_data][currency]".into(),
            request.currency.to_lowercase(),
        ),
        (
            "line_items[0][price_data][unit_amount]".into(),
            request.amount_minor.to_string(),
        ),
        (
            "line_items[0][price_data][product_data][name]".into(),
            "Order payment".into(),
        ),
        ("line_items[0][quantity]".into(), "1".into()),
    ])
}
pub fn bind_query(
    application: &str,
    request: &PaymentRequest,
    session: &str,
    url: &str,
) -> Result<String> {
    ensure!(
        session.starts_with("cs_")
            && session
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
        "Invalid session identifier"
    );
    let parsed = reqwest::Url::parse(url)?;
    ensure!(
        parsed.scheme() == "https"
            && parsed.host_str() == Some("checkout.stripe.com")
            && parsed.username().is_empty()
            && parsed.password().is_none(),
        "Invalid checkout URL"
    );
    let prefix = repository::request_match(application, request, "processing")?;
    Ok(format!(
        r#"{prefix} select $t,$p; distinct; update $p has pay_status "checkout_ready", has pay_session {}, has pay_checkout_url {};
      fetch {{"id":$p.pay_id,"session":$p.pay_session,"url":$p.pay_checkout_url}};"#,
        q(session),
        q(url)
    ))
}
pub fn session_query(application: &str, request: &PaymentRequest) -> Result<String> {
    Ok(format!(
        r#"{} $p has pay_session $session,has pay_checkout_url $url;
      fetch {{"id":$p.pay_id,"session":$session,"url":$url}};"#,
        repository::request_match(application, request, "checkout_ready")?
    ))
}
pub async fn start(
    state: &TypeDBState,
    application: &str,
    request: &PaymentRequest,
    transport: &StripeTransport,
    success: &str,
    cancel: &str,
) -> Result<Value> {
    // No provider call unless the request exists and this process wins the claim.
    let data = form(request, success, cancel)?;
    transport.require_environment(matches!(request.scope.environment, Environment::Live))?;
    let stored =
        repository::create_or_get(state, application, request, chrono::Utc::now().timestamp())
            .await?;
    if stored["status"] == "checkout_ready" {
        let response = dog_typedb::TypeDBAdapter::new(std::sync::Arc::new(state.clone()))
            .read(serde_json::json!({"query":session_query(application,request)?}))
            .await?;
        let rows = response["ok"]["answers"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid checkout lookup"))?;
        ensure!(rows.len() == 1, "Checkout unavailable");
        return Ok(rows[0]["data"].clone());
    }
    ensure!(
        stored["status"] == "requested",
        "Checkout already claimed; reconcile instead of creating again"
    );
    repository::claim(state, application, request).await?;
    let session = transport
        .request(
            Method::POST,
            "/checkout/sessions",
            &format!("{}-checkout", request.scope.idempotency_key()?),
            &data,
        )
        .await?;
    let id = session["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing checkout session"))?;
    let url = session["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing checkout URL"))?;
    let mut rows = access::write_one(state, &bind_query(application, request, id, url)?).await?;
    Ok(rows.remove(0))
}
/// Re-fetch from Stripe before constructing this query. Signature alone is insufficient.
pub fn paid_query(
    application: &str,
    request: &PaymentRequest,
    session: &Value,
    now: i64,
) -> Result<String> {
    let key = request.scope.idempotency_key()?;
    ensure!(
        session["mode"] == "payment" && session["payment_status"] == "paid",
        "Payment not complete"
    );
    ensure!(
        session["amount_total"].as_i64() == Some(request.amount_minor)
            && session["currency"] == request.currency.to_lowercase(),
        "Payment amount/currency mismatch"
    );
    ensure!(
        session["livemode"].as_bool()
            == Some(matches!(request.scope.environment, Environment::Live)),
        "Payment environment mismatch"
    );
    ensure!(
        session["client_reference_id"] == key && session["metadata"]["payment_operation"] == key,
        "Payment reference mismatch"
    );
    let id = session["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing session identifier"))?;
    let prefix = repository::request_match(application, request, "checkout_ready")?;
    Ok(format!(
        r#"{prefix} $p has pay_session {}; select $t,$p; distinct;
      update $p has pay_status "paid";
      insert $e isa pay_event, has pay_id {}, has pay_event_type "payment.paid", has pay_created {now},
       has pay_delivery_status "pending", has pay_attempts 0, has pay_due {now}, has pay_lease "";
      (request:$p,event:$e) isa pay_event_owner;
      fetch {{"id":$p.pay_id,"status":$p.pay_status}};"#,
        q(id),
        q(&format!("{key}-paid"))
    ))
}
pub async fn reconcile(
    state: &TypeDBState,
    application: &str,
    request: &PaymentRequest,
    transport: &StripeTransport,
    session_id: &str,
) -> Result<Value> {
    ensure!(
        session_id.starts_with("cs_")
            && session_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
        "Invalid session identifier"
    );
    // Re-authorize before provider I/O; stored session binding is checked by the final write.
    let stored = repository::lookup(state, application, request)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Payment unavailable"))?;
    if stored["status"] == "paid" {
        return Ok(stored);
    }
    transport.require_environment(matches!(request.scope.environment, Environment::Live))?;
    let session = transport
        .request(
            Method::GET,
            &format!("/checkout/sessions/{session_id}"),
            "",
            &[],
        )
        .await?;
    let mut rows = access::write_one(
        state,
        &paid_query(
            application,
            request,
            &session,
            chrono::Utc::now().timestamp(),
        )?,
    )
    .await?;
    Ok(rows.remove(0))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_is_not_a_purchase_and_amounts_are_bound() {
        let r:PaymentRequest=serde_json::from_value(serde_json::json!({"scope":{"merchant_id":"m","product_id":"shop","environment":"sandbox","operation_id":"o"},"amount_minor":100,"currency":"USD","currency_exponent":2,"reference":"cart"})).unwrap();
        let fields = form(
            &r,
            "https://seyfr.jitpomi.com/shop",
            "https://seyfr.jitpomi.com/shop",
        )
        .unwrap();
        assert!(fields.contains(&("mode".into(), "payment".into())));
        assert!(paid_query(
            "app",
            &r,
            &serde_json::json!({"mode":"setup","payment_status":"paid"}),
            1
        )
        .is_err());
        assert!(form(&r, "http://unsafe.example", "https://valid.example").is_err());
    }
}
