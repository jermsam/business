//! Real development-server validation; schema and synthetic fixtures are rolled back.
use super::{
    operation::{Environment, OperationScope},
    repository,
    requests::PaymentRequest,
};
use crate::access;

#[tokio::test]
#[ignore = "Requires development TypeDB; uses an uncommitted schema transaction"]
async fn typedb_payment_queries_rollback() -> anyhow::Result<()> {
    dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
    let db = std::env::var("TYPEDB_DB")?;
    anyhow::ensure!(db == "business_dev", "Development database required");
    let driver = dog_typedb::TypeDBDriverFactory::connect(
        &std::env::var("TYPEDB_ADDR")?,
        &std::env::var("TYPEDB_USERNAME")?,
        &std::env::var("TYPEDB_PASSWORD")?,
        true,
    )
    .await?;
    let tx = driver
        .transaction_with_options(
            &db,
            typedb_driver::TransactionType::Schema,
            typedb_driver::TransactionOptions::new()
                .transaction_timeout(std::time::Duration::from_secs(120)),
        )
        .await?;
    tx.query(include_str!("../../../migrations/009-payment-platform.tql"))
        .await?;
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let merchant = format!("pay-test-{tag}");
    let app = format!("pay-app-{tag}");
    tx.query(format!(r#"insert $m isa company, has company_domain "{tag}.example.com", has biz_id "{merchant}", has biz_revision "initial";
      $a isa pay_application, has pay_id "{app}";
      $g isa pay_grant, links (merchant:$m,application:$a), has pay_id "grant-{tag}", has pay_product "shop", has pay_environment "sandbox", has pay_active true, has pay_revision "initial";"#)).await?;
    let request = PaymentRequest {
        scope: OperationScope {
            merchant_id: merchant,
            product_id: "shop".into(),
            environment: Environment::Sandbox,
            operation_id: "order-1".into(),
        },
        amount_minor: 100,
        currency: "USD".into(),
        currency_exponent: 2,
        reference: "cart-1".into(),
    };
    let created = access::documents(
        &tx,
        &access::guard_write(&repository::create_query(&app, &request, 100)?)?,
    )
    .await?;
    anyhow::ensure!(created.len() == 1, "Expected one request");
    let replay = access::documents(&tx, &repository::lookup_query(&app, &request)?).await?;
    repository::verify_replay(&request, &replay[0])?;
    let mut changed = request.clone();
    changed.amount_minor = 101;
    anyhow::ensure!(
        repository::verify_replay(&changed, &replay[0]).is_err(),
        "Changed payload accepted"
    );
    anyhow::ensure!(
        access::documents(&tx, &repository::lookup_query("unknown-app", &request)?)
            .await?
            .is_empty(),
        "Application isolation failed"
    );
    let mut other = request.clone();
    other.scope.environment = Environment::Live;
    anyhow::ensure!(
        access::documents(&tx, &repository::lookup_query(&app, &other)?)
            .await?
            .is_empty(),
        "Environment isolation failed"
    );
    let claim = access::guard_write(&repository::claim_query(&app, &request)?)?;
    anyhow::ensure!(
        access::documents(&tx, &claim).await?.len() == 1,
        "First claim failed"
    );
    anyhow::ensure!(
        access::documents(&tx, &claim).await?.is_empty(),
        "Claim replay succeeded"
    );
    let events =
        access::documents(&tx, "match $e isa pay_event; fetch {\"id\":$e.pay_id};").await?;
    anyhow::ensure!(events.len() == 1, "Expected one atomic event");
    let session_id = "cs_test_fixture";
    let bind = super::checkout::bind_query(
        &app,
        &request,
        session_id,
        "https://checkout.stripe.com/c/pay/test",
    )?;
    anyhow::ensure!(
        access::documents(&tx, &access::guard_write(&bind)?)
            .await?
            .len()
            == 1,
        "Checkout binding failed"
    );
    let saved_session =
        access::documents(&tx, &super::checkout::session_query(&app, &request)?).await?;
    anyhow::ensure!(
        saved_session.len() == 1 && saved_session[0]["session"] == session_id,
        "Checkout replay lookup failed"
    );
    let session = serde_json::json!({"id":session_id,"mode":"payment","payment_status":"paid","amount_total":100,"currency":"usd","livemode":false,
      "client_reference_id":request.scope.idempotency_key()?,"metadata":{"payment_operation":request.scope.idempotency_key()?}});
    let paid = access::guard_write(&super::checkout::paid_query(&app, &request, &session, 101)?)?;
    anyhow::ensure!(
        access::documents(&tx, &paid).await?.len() == 1,
        "Paid transition failed"
    );
    anyhow::ensure!(
        access::documents(&tx, &paid).await?.is_empty(),
        "Duplicate paid event"
    );
    let event_id = format!("{}-paid", request.scope.idempotency_key()?);
    let delivery = access::guard_write(&super::delivery::claim_query(&event_id, "lease-1", 102))?;
    anyhow::ensure!(
        access::documents(&tx, &delivery).await?.len() == 1,
        "Delivery claim failed"
    );
    anyhow::ensure!(
        access::documents(&tx, &delivery).await?.is_empty(),
        "Active delivery lease replayed"
    );
    let stale = access::guard_write(&super::delivery::finish_query(
        &event_id,
        "wrong-lease",
        1,
        true,
        103,
    )?)?;
    anyhow::ensure!(
        access::documents(&tx, &stale).await?.is_empty(),
        "Stale acknowledgement accepted"
    );
    let retry = access::guard_write(&super::delivery::finish_query(
        &event_id, "lease-1", 1, false, 103,
    )?)?;
    anyhow::ensure!(
        access::documents(&tx, &retry).await?.len() == 1,
        "Delivery retry failed"
    );
    let claim_again =
        access::guard_write(&super::delivery::claim_query(&event_id, "lease-2", 200))?;
    anyhow::ensure!(
        access::documents(&tx, &claim_again).await?.len() == 1,
        "Retry claim failed"
    );
    let ack = access::guard_write(&super::delivery::finish_query(
        &event_id, "lease-2", 2, true, 201,
    )?)?;
    anyhow::ensure!(
        access::documents(&tx, &ack).await?.len() == 1,
        "Acknowledgement failed"
    );
    let destination = super::delivery::Destination {
        merchant: request.scope.merchant_id.clone(),
        product: "shop".into(),
        environment: "sandbox".into(),
        url: "https://fixture.example/events".into(),
        secret: "s".repeat(32),
    };
    anyhow::ensure!(
        access::documents(&tx, &super::delivery::due_query(&destination, 300))
            .await?
            .len()
            == 1,
        "Expected only initial request event due"
    );
    let request_event = format!("{}-requested", request.scope.idempotency_key()?);
    tx.query(format!(r#"match $e isa pay_event,has pay_id "{request_event}"; update $e has pay_attempts 8,has pay_delivery_status "delivering",has pay_due 1,has pay_lease "crashed";"#)).await?;
    let dead = access::guard_write(&super::delivery::exhausted_query(&request_event, 300))?;
    anyhow::ensure!(
        access::documents(&tx, &dead).await?.len() == 1,
        "Final-attempt crash did not dead-letter"
    );
    anyhow::ensure!(
        access::documents(&tx, &super::delivery::due_query(&destination, 300))
            .await?
            .is_empty(),
        "Delivered/dead event selected again"
    );
    tx.query(format!(
        r#"match $g isa pay_grant, has pay_id "grant-{tag}"; update $g has pay_active false;"#
    ))
    .await?;
    anyhow::ensure!(
        access::documents(&tx, &repository::lookup_query(&app, &request)?)
            .await?
            .is_empty(),
        "Revoked grant accepted"
    );
    tx.close().await?;
    println!("TypeDB schema, authorization, requests, checkout binding, paid-event deduplication, delivery leases/retries/acknowledgements and revocation passed; rolled back. Provider session was synthetic; no Stripe calls.");
    Ok(())
}
