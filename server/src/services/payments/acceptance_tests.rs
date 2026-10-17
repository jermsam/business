//! Disposable development database. No production database, no SEYFR connection.
use super::{
    http,
    operation::{Environment, OperationScope},
    repository,
    requests::PaymentRequest,
};
use crate::{access, typedb::TypeDBState};
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    state: Arc<TypeDBState>,
    app: String,
    token: String,
}
impl Harness {
    async fn new() -> Result<Self> {
        dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
        ensure!(
            std::env::var("TYPEDB_DB")? == "business_dev",
            "Development configuration required"
        );
        let driver = Arc::new(
            dog_typedb::TypeDBDriverFactory::connect(
                &std::env::var("TYPEDB_ADDR")?,
                &std::env::var("TYPEDB_USERNAME")?,
                &std::env::var("TYPEDB_PASSWORD")?,
                true,
            )
            .await?,
        );
        let database = format!("payments_test_{}", uuid::Uuid::new_v4().simple());
        driver.databases().create(&database).await?;
        let state = Arc::new(TypeDBState { driver, database });
        let tx = state
            .driver
            .transaction(&state.database, typedb_driver::TransactionType::Schema)
            .await?;
        tx.query("define attribute biz_id,value string; attribute biz_revision,value string; attribute company_domain,value string; entity company,owns biz_id @key,owns biz_revision @card(1),owns company_domain @card(1);").await?;
        tx.query(include_str!("../../../migrations/009-payment-platform.tql"))
            .await?;
        tx.commit().await?;
        let app = "acceptance-app".to_string();
        access::write_atomic_one(&state,r#"insert $m isa company,has biz_id "merchant",has biz_revision "initial",has company_domain "payments.example";
   $a isa pay_application,has pay_id "acceptance-app";
   $g isa pay_grant,links (merchant:$m,application:$a),has pay_id "grant",has pay_product "test-shop",has pay_environment "sandbox",has pay_active true,has pay_revision "initial";
   fetch {"id":$g.pay_id};"#).await?;
        Ok(Self {
            state,
            app,
            token: format!(
                "test-{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        })
    }
    fn request(&self, operation: &str) -> PaymentRequest {
        PaymentRequest {
            scope: OperationScope {
                merchant_id: "merchant".into(),
                product_id: "test-shop".into(),
                environment: Environment::Sandbox,
                operation_id: operation.into(),
            },
            amount_minor: 100,
            currency: "USD".into(),
            currency_exponent: 2,
            reference: "synthetic-cart".into(),
        }
    }
    fn router(&self) -> Result<axum::Router> {
        http::router(
            self.state.clone(),
            vec![http::Application {
                id: self.app.clone(),
                token_sha256: format!("{:x}", Sha256::digest(self.token.as_bytes())),
                merchant: "merchant".into(),
                product: "test-shop".into(),
                success_url: "https://example.com/payment-test-success".into(),
                cancel_url: "https://example.com/payment-test-cancel".into(),
                delivery: None,
            }],
        )
    }
    async fn cleanup(&self) -> Result<()> {
        ensure!(
            self.state.database.starts_with("payments_test_") && self.state.database.len() == 46,
            "Refuse non-fixture database deletion"
        );
        self.state
            .driver
            .databases()
            .get(&self.state.database)
            .await?
            .delete()
            .await?;
        Ok(())
    }
}
async fn call(
    router: axum::Router,
    path: &str,
    body: Value,
    token: Option<&str>,
) -> Result<(u16, Value)> {
    let mut builder = axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = router
        .oneshot(builder.body(axum::body::Body::from(body.to_string()))?)
        .await?;
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 8192).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}
fn input(operation: &str) -> Value {
    json!({"operation_id":operation,"reference":"synthetic-cart","amount_minor":100,"currency":"USD","currency_exponent":2})
}

#[derive(Clone)]
struct Receiver {
    state: Arc<TypeDBState>,
    secret: String,
    attempts: Arc<std::sync::atomic::AtomicUsize>,
}
async fn receive(
    axum::extract::State(receiver): axum::extract::State<Receiver>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    let result=async {
        let header=|name:&str|headers.get(name).and_then(|v|v.to_str().ok()).ok_or_else(||anyhow::anyhow!("Missing signature header"));
        let id=header("x-jitpomi-event")?;let timestamp=header("x-jitpomi-timestamp")?.parse::<i64>()?;
        super::delivery::verify_signature(&receiver.secret,id,timestamp,&body,header("x-jitpomi-signature")?,chrono::Utc::now().timestamp())?;
        let event:Value=serde_json::from_slice(&body)?;
        ensure!(event["type"]=="payment.paid"&&event["merchant"]=="merchant"&&event["product"]=="test-shop"&&event["environment"]=="sandbox"&&event["amount_minor"]==100&&event["currency"]=="USD","Receiver binding mismatch");
        let fingerprint=format!("{:x}",Sha256::digest(&body));
        let quoted=serde_json::to_string(id)?;
        let query=format!("match $r isa test_receipt,has pay_id {quoted};fetch {{\"fingerprint\":$r.pay_fingerprint}};");
        let tx=receiver.state.driver.transaction(&receiver.state.database,typedb_driver::TransactionType::Read).await?;
        let existing=access::documents(&tx,&query).await?;tx.close().await?;
        if existing.is_empty(){
            access::write_atomic_one(&receiver.state,&format!("insert $r isa test_receipt,has pay_id {quoted},has pay_fingerprint \"{fingerprint}\";fetch {{\"id\":$r.pay_id}};")).await?;
        }else{ensure!(existing.len()==1&&existing[0]["fingerprint"]==fingerprint,"Event identity reused with different payload");}
        Ok::<_,anyhow::Error>(())
    }.await;
    if result.is_err() {
        return StatusCode::UNAUTHORIZED;
    }
    // Simulate an acknowledgement lost AFTER durable processing.
    if receiver
        .attempts
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        == 0
    {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    }
}

#[tokio::test]
#[ignore = "Explicit Stripe sandbox, browser test-card payment, disposable TypeDB database and HTTPS receiver"]
async fn sandbox_payment_to_https_receiver() -> Result<()> {
    dotenvy::from_path(std::env::var("PAYMENTS_SANDBOX_ENV_FILE")?)?;
    let token = std::env::var("STRIPE_SECRET_KEY")?;
    ensure!(
        token.starts_with("sk_test_") || token.starts_with("rk_test_"),
        "Sandbox Stripe key required"
    );
    let h = Harness::new().await?;
    let result=async {
        let router=h.router()?;
        let operation=format!("flow-{}",uuid::Uuid::new_v4().simple());
        let request=h.request(&operation);
        let (status,session)=call(router.clone(),"/payments/v1/checkouts",input(&operation),Some(&h.token)).await?;
        ensure!(status==200,"Checkout endpoint failed ({status})");
        let id=session["session"].as_str().ok_or_else(||anyhow::anyhow!("Missing session"))?;
        let url=session["url"].as_str().ok_or_else(||anyhow::anyhow!("Missing URL"))?;
        let (_,replayed)=call(router.clone(),"/payments/v1/checkouts",input(&operation),Some(&h.token)).await?;
        ensure!(replayed["session"]==id,"Checkout replay created another session");
        println!("SANDBOX_CHECKOUT_URL={url}");
        println!("Waiting up to 8 minutes for synthetic Stripe TEST-card checkout. Never enter a real card.");
        let mut body=input(&operation);body["session_id"]=json!(id);
        let mut paid=false;
        for _ in 0..96 {
            let (code,data)=call(router.clone(),"/payments/v1/reconcile",body.clone(),Some(&h.token)).await?;
            if code==200 && data["status"]=="paid" {paid=true;break;}
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
        ensure!(paid,"Sandbox payment was not completed; no fulfilment was attempted");
        let outcomes=futures::future::join_all((0..4).map(|_|call(router.clone(),"/payments/v1/reconcile",body.clone(),Some(&h.token)))).await;
        ensure!(outcomes.iter().all(|r|matches!(r,Ok((200,_)))),"Paid reconciliation replay failed");
        let tx=h.state.driver.transaction(&h.state.database,typedb_driver::TransactionType::Schema).await?;
        tx.query("define entity test_receipt,owns pay_id @key,owns pay_fingerprint @card(1);").await?;tx.commit().await?;
        let cert=rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
        let root=reqwest::Certificate::from_der(cert.cert.der())?;
        let tls=rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![cert.cert.der().clone()],rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into())?;
        let acceptor=tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port=listener.local_addr()?.port();
        let secret=format!("{}{}",uuid::Uuid::new_v4().simple(),uuid::Uuid::new_v4().simple());
        let receiver=Receiver {state:h.state.clone(),secret:secret.clone(),attempts:Arc::new(std::sync::atomic::AtomicUsize::new(0))};
        let receiver_app=axum::Router::new().route("/events",axum::routing::post(receive)).layer(axum::extract::DefaultBodyLimit::max(8192)).with_state(receiver.clone());
        let task=tokio::spawn(async move {
            while let Ok((stream,_))=listener.accept().await {
                let acceptor=acceptor.clone();let service=receiver_app.clone();
                tokio::spawn(async move {if let Ok(tls)=acceptor.accept(stream).await {
                    let _=hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new()).serve_connection(hyper_util::rt::TokioIo::new(tls),hyper_util::service::TowerToHyperService::new(service)).await;
                }});
            }
        });
        let destination=super::delivery::Destination {merchant:"merchant".into(),product:"test-shop".into(),environment:"sandbox".into(),url:format!("https://localhost:{port}/events"),secret};
        let delivery_router=http::router_with_delivery_ca(h.state.clone(),vec![http::Application {
          id:h.app.clone(),token_sha256:format!("{:x}",Sha256::digest(h.token.as_bytes())),merchant:"merchant".into(),product:"test-shop".into(),
          success_url:"https://example.com/payment-test-success".into(),cancel_url:"https://example.com/payment-test-cancel".into(),
          delivery:Some(http::DeliveryConfig {url:destination.url.clone(),secret:destination.secret.clone()})}],root.clone())?;
        let client=reqwest::Client::builder().https_only(true).add_root_certificate(root).redirect(reqwest::redirect::Policy::none()).retry(reqwest::retry::never()).timeout(std::time::Duration::from_secs(10)).build()?;
        let event=format!("{}-paid",request.scope.idempotency_key()?);
        let (code,first)=call(delivery_router.clone(),"/payments/v1/events/dispatch",input(&operation),Some(&h.token)).await?;
        ensure!(code==200&&first["status"]=="pending","Lost ACK did not schedule retry");
        access::write_one(&h.state,&format!(r#"match $t isa company,has biz_id "merchant",has biz_revision $revision;$e isa pay_event,has pay_id "{event}";update $e has pay_due 0;fetch {{"id":$e.pay_id}};"#)).await?;
        let (code,second)=call(delivery_router.clone(),"/payments/v1/events/dispatch",input(&operation),Some(&h.token)).await?;
        ensure!(code==200&&second["status"]=="delivered","Retry did not acknowledge");
        let tx=h.state.driver.transaction(&h.state.database,typedb_driver::TransactionType::Read).await?;
        ensure!(access::documents(&tx,"match $r isa test_receipt;fetch {\"id\":$r.pay_id};").await?.len()==1,"Receiver processed twice");
        ensure!(access::documents(&tx,"match $e isa pay_event,has pay_event_type \"payment.paid\";fetch {\"id\":$e.pay_id};").await?.len()==1,"Duplicate paid event");tx.close().await?;
        let bad=client.post(&destination.url).header("x-jitpomi-event",&event).header("x-jitpomi-timestamp",chrono::Utc::now().timestamp().to_string()).header("x-jitpomi-signature","0".repeat(64)).body("{}").send().await?;
        ensure!(bad.status()==401,"Forged event accepted");
        task.abort();
        println!("PASS: authenticated checkout -> actual Stripe sandbox payment -> verified reconciliation -> atomic paid event -> signed HTTPS receiver -> lost ACK/retry -> one durable receipt. Invalid signature rejected. No SEYFR connection.");
        Ok::<_,anyhow::Error>(())
    }.await;
    h.cleanup().await?;
    result
}

#[tokio::test]
#[ignore = "Creates/deletes a disposable payments_test database using development credentials"]
async fn committed_payment_concurrency_and_authentication() -> Result<()> {
    let h = Harness::new().await?;
    let result=async {
  let router=h.router()?;
  ensure!(call(router.clone(),"/payments/v1/status",input("race"),None).await?.0==401,"Anonymous access accepted");
  ensure!(call(router.clone(),"/payments/v1/status",input("race"),Some(&"x".repeat(40))).await?.0==401,"Invalid token accepted");
  let request=h.request("race");
  let outcomes=futures::future::join_all((0..8).map(|_|repository::create_or_get(&h.state,&h.app,&request,100))).await;
  ensure!(outcomes.iter().any(|r|r.is_ok()),"No concurrent creator succeeded");
  let stored=repository::lookup(&h.state,&h.app,&request).await?.ok_or_else(||anyhow::anyhow!("Missing committed request"))?;
  repository::verify_replay(&request,&stored)?;
  let claims=futures::future::join_all((0..8).map(|_|repository::claim(&h.state,&h.app,&request))).await;
  ensure!(claims.iter().filter(|r|r.is_ok()).count()==1,"Concurrent claims did not have exactly one winner");
  ensure!(call(router.clone(),"/payments/v1/status",input("race"),Some(&h.token)).await?.0==200,"Authorized status failed");
  let mut changed=input("race");changed["amount_minor"]=json!(101);
  ensure!(call(router.clone(),"/payments/v1/status",changed,Some(&h.token)).await?.0==409,"Changed payload accepted");
  let mut injected=input("race");injected["merchant_id"]=json!("other");
  ensure!(call(router.clone(),"/payments/v1/status",injected,Some(&h.token)).await?.0==422,"Caller scope injection accepted");
  let tx=h.state.driver.transaction(&h.state.database,typedb_driver::TransactionType::Read).await?;
  ensure!(access::documents(&tx,"match $p isa pay_request; fetch {\"id\":$p.pay_id};").await?.len()==1,"Duplicate request");
  ensure!(access::documents(&tx,"match $e isa pay_event; fetch {\"id\":$e.pay_id};").await?.len()==1,"Duplicate request event");
  tx.close().await?;
  let bind=super::checkout::bind_query(&h.app,&request,"cs_test_concurrent","https://checkout.stripe.com/c/pay/test")?;
  access::write_one(&h.state,&bind).await?;
  let session=json!({"id":"cs_test_concurrent","mode":"payment","payment_status":"paid","amount_total":100,"currency":"usd","livemode":false,"client_reference_id":request.scope.idempotency_key()?,"metadata":{"payment_operation":request.scope.idempotency_key()?}});
  let paid_query=super::checkout::paid_query(&h.app,&request,&session,101)?;
  let paid=futures::future::join_all((0..8).map(|_|access::write_one(&h.state,&paid_query))).await;
  ensure!(paid.iter().filter(|r|r.is_ok()).count()==1,"Paid transition race had multiple winners");
  let event=format!("{}-paid",request.scope.idempotency_key()?);
  let leases=futures::future::join_all((0..8).map(|n|{
    let state=h.state.clone();let event=event.clone();async move {access::write_one(&state,&super::delivery::claim_query(&event,&format!("lease-{n}"),102)).await}
  })).await;
  ensure!(leases.iter().filter(|r|r.is_ok()).count()==1,"Delivery race had multiple winners");
  let winner=leases.iter().position(|r|r.is_ok()).unwrap();
  access::write_one(&h.state,&super::delivery::finish_query(&event,&format!("lease-{winner}"),1,true,103)?).await?;
  let replay_router=http::router(h.state.clone(),vec![http::Application {id:h.app.clone(),token_sha256:format!("{:x}",Sha256::digest(h.token.as_bytes())),merchant:"merchant".into(),product:"test-shop".into(),success_url:"https://example.com/success".into(),cancel_url:"https://example.com/cancel".into(),delivery:Some(http::DeliveryConfig {url:"https://127.0.0.1:1/events".into(),secret:"s".repeat(32)})}])?;
  let (code,delivery)=call(replay_router,"/payments/v1/events/dispatch",input("race"),Some(&h.token)).await?;
  ensure!(code==200&&delivery["status"]=="delivered","Delivered replay attempted another transmission");
  let tx=h.state.driver.transaction(&h.state.database,typedb_driver::TransactionType::Read).await?;
  ensure!(access::documents(&tx,"match $e isa pay_event,has pay_event_type \"payment.paid\";fetch {\"id\":$e.pay_id};").await?.len()==1,"Duplicate paid event after race");tx.close().await?;
  access::write_one(&h.state,r#"match $t isa company,has biz_id "merchant",has biz_revision $revision; $g isa pay_grant,links (merchant:$t),has pay_id "grant";update $g has pay_active false;fetch {"id":$g.pay_id};"#).await?;
  ensure!(call(router,"/payments/v1/status",input("race"),Some(&h.token)).await?.0==404,"Revoked application accepted");
  println!("Committed TypeDB: 8 concurrent creates, exactly one of 8 operation claims, one of 8 paid transitions, one of 8 delivery leases; authenticated endpoint isolation and revocation passed. Financial observation in this concurrency test was synthetic.");
  Ok::<_,anyhow::Error>(())
 }.await;
    h.cleanup().await?;
    result
}

#[tokio::test]
#[ignore = "Development credentials; disposable TypeDB database, no provider writes"]
async fn sandbox_demo_receiver_is_bound_and_deduplicated() -> Result<()> {
    let h = Harness::new().await?;
    let result = async {
        let request = h.request("receiver-check");
        let key = request.scope.idempotency_key()?;
        let now = chrono::Utc::now().timestamp();
        repository::create_or_get(&h.state, &h.app, &request, now).await?;
        access::write_one(&h.state, &repository::claim_query(&h.app, &request)?).await?;
        access::write_one(&h.state, &super::checkout::bind_query(&h.app, &request, "cs_test_receiver", "https://checkout.stripe.com/c/pay/cs_test_receiver")?).await?;
        let observation = json!({"id":"cs_test_receiver","mode":"payment","payment_status":"paid","amount_total":100,"currency":"usd","livemode":false,"client_reference_id":key,"metadata":{"payment_operation":key}});
        access::write_one(&h.state, &super::checkout::paid_query(&h.app, &request, &observation, now)?).await?;
        let secret="synthetic-receiver-signing-secret-1234567890";
        let router=http::router(h.state.clone(),vec![http::Application{id:h.app.clone(),token_sha256:format!("{:x}",Sha256::digest(h.token.as_bytes())),merchant:"merchant".into(),product:"test-shop".into(),success_url:"https://example.com/ok".into(),cancel_url:"https://example.com/cancel".into(),delivery:Some(http::DeliveryConfig{url:"https://jitpomi-business-validation.onrender.com/payments/sandbox/receiver".into(),secret:secret.into()})}])?;
        let event=format!("{key}-paid");
        let payload=json!({"id":event,"type":"payment.paid","operation":key,"reference":"synthetic-cart","merchant":"merchant","product":"test-shop","environment":"sandbox","amount_minor":100,"currency":"USD","currency_exponent":2});
        for attempt in 0..3 {
            let body=serde_json::to_vec(&payload)?;
            let signature=if attempt==2 {"0".repeat(64)}else{super::delivery::signature(secret,&event,now,&body)?};
            let response=router.clone().oneshot(axum::http::Request::builder().method("POST").uri("/payments/sandbox/receiver").header("x-jitpomi-event",&event).header("x-jitpomi-timestamp",now.to_string()).header("x-jitpomi-signature",signature).body(axum::body::Body::from(body))?).await?;
            ensure!(response.status().as_u16()==if attempt==2{401}else{200},"Receiver attempt {attempt} returned {}",response.status());
        }
        let tx=h.state.driver.transaction(&h.state.database,typedb_driver::TransactionType::Read).await?;
        ensure!(access::documents(&tx,"match $r isa pay_sandbox_receipt;fetch {\"id\":$r.pay_id};").await?.len()==1,"Duplicate demo receipt");
        tx.close().await?;
        Ok::<_,anyhow::Error>(())
    }.await;
    h.cleanup().await?;
    result
}
