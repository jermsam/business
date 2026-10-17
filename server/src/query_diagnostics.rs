#[tokio::test]
#[ignore = "Read-only business_dev query timing diagnostics"]
async fn hosted_policy_query_diagnostics() {
    use crate::access::documents;
    use typedb_driver::TransactionType;
    use std::time::{Duration,Instant};
    let (app, _)=build().await.unwrap();
    let state=app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
    assert_eq!(state.database,"business_dev");
    let schema=state.driver.databases().get(&state.database).await.unwrap().schema().await.unwrap();
    println!("POLICY_SCHEMA split_grants={}",schema.contains("fun biz_direct_effect"));
    let tx=state.driver.transaction_with_options(&state.database,TransactionType::Read,typedb_driver::TransactionOptions::new().transaction_timeout(Duration::from_secs(10))).await.unwrap();
    let ids=documents(&tx,r#"match $project isa biz_project, has name "Soak project"; (tenant:$t,product:$p,project:$project) isa biz_project_owner; (tenant:$t,person:$u) isa biz_membership; limit 1; fetch {"u":$u.biz_id,"t":$t.biz_id,"p":$p.biz_id,"project":$project.biz_id};"#).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2),tx.close()).await;
    let row=&ids[0];
    let scope=format!(r#"match $u isa user, has biz_id "{}"; $t isa company, has biz_id "{}"; $p isa biz_product, has biz_id "{}"; $project isa biz_project, has biz_id "{}"; let $now={}; "#,row["u"].as_str().unwrap(),row["t"].as_str().unwrap(),row["p"].as_str().unwrap(),row["project"].as_str().unwrap(),crate::auth::local::now());
    for (name,expression) in [
        ("identity","true"),
        ("member","biz_member($u,$t,$now)"),
        ("entitlement","biz_entitled($t,$p,$now)"),
        ("allow","biz_effect($u,$t,$project,\"create\",\"allow\",$now)"),
        ("deny","biz_effect($u,$t,$project,\"create\",\"deny\",$now)"),
        ("authorized","biz_authorized($u,$t,$project,\"create\",$now)"),
    ] {
        let tx=state.driver.transaction_with_options(&state.database,TransactionType::Read,typedb_driver::TransactionOptions::new().transaction_timeout(Duration::from_secs(10))).await.unwrap();
        let started=Instant::now();
        let result=tokio::time::timeout(Duration::from_secs(10),documents(&tx,&format!("{scope} select $u,$t,$p,$project,$now; match let $ok={expression}; fetch {{\"ok\":$ok}};"))).await;
        println!("POLICY_DIAGNOSTIC {name} ms={} completed={} succeeded={}",started.elapsed().as_millis(),result.is_ok(),matches!(result,Ok(Ok(_))));
        let _ = tokio::time::timeout(Duration::from_secs(2),tx.close()).await;
    }
}
