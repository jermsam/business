#[tokio::test]
#[ignore = "Requires business_dev; verifies protected transaction rollback"]
async fn hosted_protected_write_atomicity() {
    use crate::access::{documents, write_one};
    use typedb_driver::TransactionType;
    let (app, _) = build().await.unwrap();
    let state = app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
    assert_eq!(state.database, "business_dev");
    let adapter = dog_typedb::TypeDBAdapter::new(state.clone());
    let tag = uuid::Uuid::new_v4();
    let tid = uuid::Uuid::new_v4();
    let first = format!("atomic-first-{tag}");
    let second = format!("atomic-second-{tag}");
    adapter.write(json!({"query":format!(r#"insert
        $t isa company, has company_domain "atomic-{tag}.example.com", has biz_id "{tid}", has biz_revision "initial";
        $a isa biz_product, has biz_id "{first}", has name "original";
        $b isa biz_product, has biz_id "{second}", has name "original";"#)})).await.unwrap();
    let scope = format!(r#"match $t isa company, has biz_id "{tid}", has biz_revision $revision;"#);
    let both = format!(r#"{scope} $p isa biz_product; {{ $p has biz_id "{first}"; }} or {{ $p has biz_id "{second}"; }};"#);
    let err = write_one(&state, &format!(r#"{both} update $p has name "ambiguous"; fetch {{"id":$p.biz_id}};"#)).await.unwrap_err();
    assert!(err.to_string().contains("Ambiguous mutation"));
    // A mutation can produce zero results AFTER it has buffered a change.
    let one = format!(r#"{scope} $p isa biz_product, has biz_id "{first}";"#);
    let err = write_one(&state, &format!(r#"{one} update $p has name "hidden-change"; match $p has name "impossible"; fetch {{"id":$p.biz_id}};"#)).await.unwrap_err();
    assert!(err.to_string().contains("operation not allowed"));
    let read = state.driver.transaction(&state.database, TransactionType::Read).await.unwrap();
    let rows = documents(&read, &format!(r#"{both} fetch {{"name":$p.name,"revision":$t.biz_revision}};"#)).await.unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows { assert_eq!(row["name"], "original"); assert_eq!(row["revision"], "initial"); }
    read.close().await.unwrap();
    let rows = write_one(&state, &format!(r#"{one} update $p has name "committed"; fetch {{"id":$p.biz_id}};"#)).await.unwrap();
    assert_eq!(rows.len(),1);
    let read = state.driver.transaction(&state.database, TransactionType::Read).await.unwrap();
    let rows = documents(&read, &format!(r#"{one} fetch {{"name":$p.name,"revision":$t.biz_revision}};"#)).await.unwrap();
    assert_eq!(rows[0]["name"], "committed");
    assert_ne!(rows[0]["revision"], "initial");
    read.close().await.unwrap();
}

#[tokio::test]
#[ignore = "Requires business_dev; malformed fixtures are rolled back"]
async fn hosted_migration_rejects_invalid_identity_and_ownership() {
    use crate::access::{backfill, documents};
    use typedb_driver::TransactionType;
    let (app, _) = build().await.unwrap();
    let state = app.get::<std::sync::Arc<crate::typedb::TypeDBState>>("typedb").unwrap();
    assert_eq!(state.database, "business_dev");
    let tag = uuid::Uuid::new_v4();
    let tx = state.driver.transaction(&state.database, TransactionType::Write).await.unwrap();
    tx.query(format!(r#"insert $u isa user, has email "bad-{tag}@example.com", has biz_id "invalid-{tag}";"#)).await.unwrap();
    assert!(backfill(&tx).await.is_err());
    tx.close().await.unwrap();
    let tx = state.driver.transaction(&state.database, TransactionType::Write).await.unwrap();
    tx.query(format!(r#"insert $r isa business_record, has record_id "{tag}", has name "orphan", has email "orphan-{tag}@example.com", has company_domain "orphan-{tag}.example.com", has biz_person_id "{tag}", has biz_tenant_id "{tag}";"#)).await.unwrap();
    let error = backfill(&tx).await.unwrap_err();
    assert!(error.to_string().contains("Dangling stable private ownership"), "{error:#}");
    tx.close().await.unwrap();
    let read = state.driver.transaction(&state.database, TransactionType::Read).await.unwrap();
    assert!(documents(&read, &format!(r#"match $r isa business_record, has record_id "{tag}"; fetch {{"id":$r.record_id}};"#)).await.unwrap().is_empty());
    assert!(documents(&read, &format!(r#"match $u isa user, has email "bad-{tag}@example.com"; fetch {{"email":$u.email}};"#)).await.unwrap().is_empty());
    read.close().await.unwrap();
}
