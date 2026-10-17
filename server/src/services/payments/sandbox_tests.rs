//! Explicit sandbox-only API smoke test. Creates/retrieves/expires a session; no charge.
#[tokio::test]
#[ignore = "Requires explicitly selected Stripe sandbox environment file"]
async fn sandbox_one_time_checkout() -> anyhow::Result<()> {
    use super::{
        checkout,
        operation::{Environment, OperationScope},
        requests::PaymentRequest,
        stripe::StripeTransport,
    };
    dotenvy::from_path(std::env::var("PAYMENTS_SANDBOX_ENV_FILE")?)?;
    let token = std::env::var("STRIPE_SECRET_KEY")?;
    anyhow::ensure!(
        token.starts_with("sk_test_") || token.starts_with("rk_test_"),
        "Sandbox key required"
    );
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(std::time::Duration::from_secs(25))
        .build()?;
    let transport = StripeTransport::new(http, false);
    let request = PaymentRequest {
        scope: OperationScope {
            merchant_id: "sandbox-fixture".into(),
            product_id: "shop-test".into(),
            environment: Environment::Sandbox,
            operation_id: uuid::Uuid::new_v4().to_string(),
        },
        amount_minor: 100,
        currency: "USD".into(),
        currency_exponent: 2,
        reference: "synthetic-cart".into(),
    };
    let fields = checkout::form(
        &request,
        "https://seyfr.jitpomi.com/shop",
        "https://seyfr.jitpomi.com/shop",
    )?;
    let key = request.scope.idempotency_key()?;
    let session = transport
        .request(reqwest::Method::POST, "/checkout/sessions", &key, &fields)
        .await?;
    let id = session["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing sandbox session"))?;
    let observed = transport
        .request(
            reqwest::Method::GET,
            &format!("/checkout/sessions/{id}"),
            "",
            &[],
        )
        .await;
    // Cleanup first, even if retrieval/verification failed.
    let expired = transport
        .request(
            reqwest::Method::POST,
            &format!("/checkout/sessions/{id}/expire"),
            &format!("{key}-expire"),
            &[],
        )
        .await?;
    let observed = observed?;
    anyhow::ensure!(
        observed["mode"] == "payment"
            && observed["livemode"] == false
            && observed["amount_total"] == 100
            && observed["payment_status"] == "unpaid",
        "Unexpected checkout configuration"
    );
    anyhow::ensure!(
        checkout::paid_query("sandbox-app", &request, &observed, 1).is_err(),
        "Unpaid checkout accepted"
    );
    anyhow::ensure!(expired["status"] == "expired", "Sandbox cleanup failed");
    println!("Stripe sandbox one-time Checkout created, retrieved, verified unpaid, and expired. No payment made.");
    Ok(())
}
