#[tokio::test]
#[ignore = "Requires business_dev; real driver disconnect and server timeout tests"]
async fn hosted_connection_loss_and_timeout_recovery() {
    use crate::access::{documents, guard_write};
    use std::time::Duration;
    use typedb_driver::{TransactionOptions, TransactionType};
    let (app, _) = build().await.unwrap();
    let verifier = app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
    assert_eq!(verifier.database, "business_dev");
    println!("TypeDB server {}", verifier.driver.server_version().await.unwrap().version());
    let adapter = dog_typedb::TypeDBAdapter::new(verifier.clone());
    for scenario in ["timeout", "disconnect", "commit-race"] {
        let (victim_app, _) = build().await.unwrap();
        let victim = victim_app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
        let id = uuid::Uuid::new_v4();
        adapter.write(json!({"query":format!(r#"insert $t isa company, has biz_id "{id}", has company_domain "recovery-{id}.example.com", has biz_revision "initial"; $p isa biz_product, has biz_id "{id}", has name "before";"#)})).await.unwrap();
        let scope = format!(r#"match $t isa company, has biz_id "{id}", has biz_revision $v; $p isa biz_product, has biz_id "{id}";"#);
        let tx = victim.driver.transaction_with_options(&victim.database, TransactionType::Write,
            TransactionOptions::new().transaction_timeout(Duration::from_secs(if scenario == "timeout" { 2 } else { 30 }))).await.unwrap();
        documents(&tx, &guard_write(&format!(r#"{scope} update $p has name "after"; fetch {{"id":$p.biz_id}};"#)).unwrap()).await.unwrap();
        let committed = match scenario {
            "timeout" => {
                tokio::time::sleep(Duration::from_secs(3)).await;
                assert!(tx.commit().await.is_err()); false
            }
            "disconnect" => {
                victim.driver.force_close().unwrap();
                assert!(tx.commit().await.is_err()); false
            }
            _ => {
                // Cut the real driver's connections concurrently with commit. Either
                // atomic outcome is legal; an error must not be interpreted as rollback.
                let (commit, _) = tokio::join!(tx.commit(), async {
                    tokio::task::yield_now().await;
                    victim.driver.force_close().unwrap();
                });
                commit.is_ok()
            }
        };
        let read = verifier.driver.transaction(&verifier.database, TransactionType::Read).await.unwrap();
        let rows = documents(&read, &format!(r#"{scope} fetch {{"name":$p.name,"revision":$t.biz_revision}};"#)).await.unwrap();
        assert_eq!(rows.len(),1);
        let changed = rows[0]["name"] == "after";
        assert_eq!(changed, rows[0]["revision"] != "initial", "No partial commit");
        if scenario != "commit-race" { assert!(!changed); }
        if committed { assert!(changed); }
        read.close().await.unwrap();
        // A fresh application connection must still perform a protected write.
        let (restarted, _) = build().await.unwrap();
        let state = restarted.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
        crate::access::write_one(&state, &format!(r#"{scope} update $p has name "recovered"; fetch {{"id":$p.biz_id}};"#)).await.unwrap();
        println!("recovery scenario={scenario} commit_ack={committed} committed_state={changed} recovery=passed");
    }
}
