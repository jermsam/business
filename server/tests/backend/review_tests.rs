//! Focused review reproduction through the registered authentication/data hooks.
use super::*;
use anyhow::{ensure, Result};
use futures::FutureExt;
use serde_json::json;
use std::sync::Arc;
use typedb_driver::TransactionType;

#[tokio::test]
#[ignore = "Real TypeDB; disposable database; concurrent review reproduction"]
async fn concurrent_review_reproduction() -> Result<()> {
    dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
    ensure!(
        std::env::var("TYPEDB_DB")? == "business_dev",
        "Development credentials only"
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
    let database = format!("business_backend_test_{}", uuid::Uuid::new_v4().simple());
    driver.databases().create(&database).await?;
    let outcome = std::panic::AssertUnwindSafe(async {
        let tx = driver.transaction(&database, TransactionType::Schema).await?;
        for source in [SCHEMA, FUNCTIONS, SCHEMA_POLICY, BILLING] { tx.query(source).await?; }
        tx.commit().await?;
        let state = Arc::new(server::typedb::TypeDBState { driver: driver.clone(), database: database.clone() });
        let trusted = dog_typedb::TypeDBAdapter::new(state.clone());
        trusted.write(json!({"query": include_str!("fixtures.tql").replace("@@password_hash@@", &bcrypt::hash("rebuild-test-password", 4)?)})).await?;
        let (app, http) = build_with_state(state, "focused-review-test-secret-more-than-thirty-two-bytes".into())?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        struct Server(tokio::task::JoinHandle<()>);
        impl Drop for Server { fn drop(&mut self) { self.0.abort(); } }
        let _server = Server(tokio::spawn(async move { axum::serve(listener, router(http)).await.unwrap(); }));
        let carol = super::schema_policy_tests::login(&reqwest::Client::new(), &base, "tenant-a", "carol@example.com").await?;
        let mut params = server::BusinessParams::default();
        params.headers.insert("authorization".into(), format!("Bearer {}", carol["accessToken"].as_str().unwrap()));
        let mut outcomes = std::collections::BTreeMap::new();
        for round in 0..3 {
            let results = futures::future::join_all((0..4).map(|index| {
                let service = app.service("data").unwrap();
                let params = params.clone();
                let id = format!("load-review-{round}-{index}");
                let query = super::schema_policy_tests::render("review-grant", &json!({
                    "grant_id":"direct-read", "review_id":id,
                    "outcome":"certified", "reason":"Synthetic review concurrency regression"
                }), &carol["user"]);
                async move {
                    let start = std::time::Instant::now();
                    let result = service.custom(dog_core::TenantContext::new("tenant-a"), "write", Some(json!({"query":query})), params).await;
                    match &result {
                        Ok(_) => eprintln!("Review round={round} client={index} success elapsed={:?}", start.elapsed()),
                        Err(error) => eprintln!("Synthetic review round={round} client={index} elapsed={:?}: {error:#}", start.elapsed()),
                    }
                    (id, result)
                }
            })).await;
            let mut successes = 0;
            for (id, result) in results {
                match result {
                    Ok(value) => {
                        ensure!(value["ok"]["answers"].as_array().is_some_and(|a|a.len()==1), "Review response missing");
                        ensure!(value["ok"]["answers"][0]["data"]["review_id"]==id, "Wrong review returned");
                        outcomes.insert(id, true);
                        successes += 1;
                    }
                    Err(error) => {
                        let code = error.downcast_ref::<dog_core::DogError>().map(|e|e.code());
                        let driver_code = error.chain().find_map(|e|e.downcast_ref::<typedb_driver::Error>().map(|e|e.code()));
                        eprintln!("Synthetic review conflict driver code: {driver_code:?}");
                        ensure!(code == Some(409), "Review failed unexpectedly: {error:#}");
                        outcomes.insert(id, false);
                    }
                }
            }
            ensure!(successes > 0, "All concurrent reviews failed");
        }
        // The actual HTTP route must work too, with its normal admission and
        // transport deadlines. No extra timeout allowance or automatic replay.
        let client = reqwest::Client::new();
        for batch in [1, 2] {
            let results = futures::future::join_all((0..batch).map(|index| {
                let id = format!("http-review-{batch}-{index}");
                let query = super::schema_policy_tests::render("review-grant", &json!({
                    "grant_id":"direct-read", "review_id":id, "outcome":"certified", "reason":"HTTP review regression"
                }), &carol["user"]);
                let request = client.post(format!("{base}/data")).header("x-tenant-id", "tenant-a")
                    .header("x-service-method", "write").bearer_auth(carol["accessToken"].as_str().unwrap())
                    .json(&json!({"query":query}));
                async move {
                    let start = std::time::Instant::now();
                    let response = request.send().await?;
                    let status = response.status().as_u16();
                    let body:serde_json::Value = response.json().await?;
                    eprintln!("HTTP review {id}: status={status}, elapsed={:?}", start.elapsed());
                    Ok::<_, anyhow::Error>((id, status, body))
                }
            })).await;
            let mut successes = 0;
            for result in results {
                let (id, status, body) = result?;
                ensure!(status==200 || (batch>1 && status==409), "HTTP review failed: status={status} body={body}");
                if status==200 {
                    ensure!(body["ok"]["answers"][0]["data"]["review_id"]==id, "Wrong HTTP review returned");
                    successes += 1;
                }
                outcomes.insert(id, status==200);
            }
            ensure!(successes > 0, "No HTTP review succeeded");
        }
        let history = trusted.read(json!({"query":"match $r isa access_review;fetch {'id':$r.review_id};"})).await?;
        let persisted:std::collections::BTreeSet<_> = history["ok"]["answers"].as_array().unwrap().iter()
            .map(|row|row["data"]["id"].as_str().unwrap().to_owned()).collect();
        for (id, success) in &outcomes {
            ensure!(persisted.contains(id)==*success, "Unreconciled review outcome for {id}: expected persisted={success}");
        }
        ensure!(persisted.len()==outcomes.values().filter(|ok|**ok).count(), "Extra review history persisted");
        Ok::<_, anyhow::Error>(())
    }).catch_unwind().await;
    driver.databases().get(&database).await?.delete().await?;
    match outcome {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
