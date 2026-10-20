//! Domain RBAC regression matrix, independent of transport/query contracts.
use super::*;
use anyhow::{ensure, Result};
use futures::FutureExt;
use server::database::documents;
use typedb_driver::{Transaction, TransactionType};

async fn allowed(
    tx: &Transaction,
    identity: &str,
    tenant: &str,
    resource: &str,
    action: &str,
    now: &str,
) -> Result<bool> {
    // Fixture identifiers only. Public query bindings are validated separately.
    let rows = documents(
        tx,
        &format!(
            r#"match
      $i isa identity, has identity_id "{identity}";
      $t isa tenant, has tenant_id "{tenant}";
      $r isa resource, has resource_id "{resource}";
      let $ok = is_permitted($i,$t,$r,"{action}",{now});
      fetch {{"allowed":$ok}};"#
        ),
    )
    .await?;
    ensure!(rows.len() == 1, "Expected one decision, got {rows:?}");
    rows[0]["allowed"]
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("Invalid decision"))
}

#[tokio::test]
#[ignore = "Creates and deletes an isolated business_backend_test UUID database; never resets the configured database"]
async fn real_typedb_model_and_policy_matrix() -> Result<()> {
    dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
    ensure!(
        std::env::var("TYPEDB_DB")? == "business_dev",
        "Development credentials only"
    );
    let driver = std::sync::Arc::new(
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
    let result=std::panic::AssertUnwindSafe(async {
        let schema = driver.transaction(&database, TransactionType::Schema).await?;
        schema.query(SCHEMA).await?;
        schema.query(FUNCTIONS).await?;
        schema.commit().await?;
        let seed = driver.transaction(&database, TransactionType::Write).await?;
        seed.query(include_str!("fixtures.tql").replace("@@password_hash@@", &bcrypt::hash("rebuild-test-password", 4)?)).await?;
        seed.commit().await?;
        // Schema, not application code, rejects unaccountable grants.
        for bad in [
            r#"$g isa access_grant,links(tenant:$t,grantee:$i,resource:$r,grantor:$by),has grant_id "bad-abstract",has permission_level "read",has effect "allow",has start_at 2025-01-01T00:00:00;"#,
            r#"$g isa exception_access_grant,links(tenant:$t,grantee:$i,resource:$r,grantor:$by),has grant_id "bad-reason",has permission_level "read",has effect "allow",has start_at 2025-01-01T00:00:00;"#,
            r#"$g isa policy_access_grant,links(tenant:$t,grantee:$i,resource:$r,grantor:$by),has grant_id "bad-policy",has permission_level "read",has effect "allow",has start_at 2025-01-01T00:00:00;"#,
            r#"$g isa exception_access_grant,links(tenant:$t,grantee:$i,resource:$r,grantor:$by),has grant_id "bad-permission",has permission_level "superadmin",has effect "allow",has reason "Invalid",has start_at 2025-01-01T00:00:00;"#,
        ] {
            let tx=driver.transaction(&database,TransactionType::Write).await?;
            let query=format!(r#"match $t isa tenant,has tenant_id "tenant-a";$i isa identity,has identity_id "bob";$r isa resource,has resource_id "files";$by isa identity,has identity_id "alice";insert {bad}"#);
            if tx.query(query).await.is_ok() {
                ensure!(tx.commit().await.is_err(),"Schema accepted an unaccountable grant");
            } else {let _=tx.close().await;}
        }
        let tx = driver.transaction(&database, TransactionType::Read).await?;
        let now = "2026-10-20T00:00:00";
        // Regression proof of the actual enforcement boundary: defining RBAC
        // functions does not turn them into mandatory row-level security.
        // This query deliberately omits every authorization function. It runs
        // only in this generated disposable database, never through public HTTP.
        let unscoped = documents(&tx,
            r#"match $r isa resource; fetch {"id":$r.resource_id};"#).await?;
        ensure!(unscoped.iter().any(|r| r["id"] == "foreign"),
            "Recheck boundary: raw TypeQL behavior changed");
        ensure!(!allowed(&tx,"bob","tenant-a","foreign","read",now).await?,
            "TypeQL policy allowed a foreign resource");
        // Even a schema-valid write is not automatically checked against the
        // caller's permissions. Demonstrate and roll back, preserving fixtures.
        let bypass = driver.transaction(&database,TransactionType::Write).await?;
        bypass.query(r#"match $r isa resource,has resource_id "foreign";
            update $r has name "Unauthorized mutation proof";"#).await?;
        let changed=documents(&bypass,r#"match $r isa resource,has resource_id "foreign";
            fetch {"name":$r.name};"#).await?;
        ensure!(changed[0]["name"]=="Unauthorized mutation proof",
            "Recheck boundary: unguarded writes changed behavior");
        bypass.close().await?;
        for (who, tenant, resource, action, expected) in [
            ("bob","tenant-a","files","read",true),
            ("bob","tenant-a","files","write",true),
            ("bob","tenant-a","files","admin",false),
            ("bob","tenant-b","files","read",false),
            ("bob","tenant-a","foreign","read",false),
            ("alice","tenant-a","files","read",false),
            ("bot","tenant-a","files","write",true),
            ("bot","tenant-a","files","admin",false),
        ] { ensure!(allowed(&tx,who,tenant,resource,action,now).await? == expected, "Wrong decision for {who}/{tenant}/{resource}/{action}"); }
        ensure!(!allowed(&tx,"bob","tenant-a","files","write","2027-01-01T00:00:00").await?, "Expiry boundary failed");
        tx.close().await?;
        let tx=driver.transaction(&database,TransactionType::Write).await?;
        tx.query(r#"match $v isa policy_revision,has revision_id "member-files-v1";update $v has required_role "owner";"#).await?;
        let excess=documents(&tx,r#"match $t isa tenant,has tenant_id "tenant-a";let $i,$g in excess_policy_access($t,2026-10-20T00:00:00);fetch {"identity":$i.identity_id,"grant":$g.grant_id};"#).await?;
        ensure!(excess.iter().any(|r|r["identity"]=="bob"&&r["grant"]=="direct-read"),"Policy drift was not detected");
        tx.close().await?;

        // Roll back each state change independently; historical grants survive.
        for mutation in [
            r#"match $i isa identity,has identity_id "bob"; update $i has status "suspended";"#,
            r#"match $t isa tenant,has tenant_id "tenant-a"; update $t has status "suspended";"#,
            r#"match $r isa resource,has resource_id "files"; update $r has status "suspended";"#,
            r#"match $i isa identity,has identity_id "bob";$m isa membership,links(member:$i); update $m has end_at 2026-10-20T00:00:00;"#,
        ] {
            let tx = driver.transaction(&database, TransactionType::Write).await?;
            tx.query(mutation).await?;
            ensure!(!allowed(&tx,"bob","tenant-a","files","read",now).await?, "Kill switch bypass");
            tx.close().await?;
        }
        let tx = driver.transaction(&database, TransactionType::Write).await?;
        tx.query(r#"match $i isa identity,has identity_id "bob";$t isa tenant,has tenant_id "tenant-a";$r isa resource,has resource_id "files";$actor isa identity,has identity_id "alice";
          insert $g isa exception_access_grant,links(tenant:$t,grantee:$i,resource:$r,grantor:$actor),has grant_id "deny-read",has permission_level "read",has effect "deny",has start_at 2025-01-01T00:00:00,has reason "Restricted";"#).await?;
        ensure!(!allowed(&tx,"bob","tenant-a","files","read",now).await?, "Explicit deny lost to team allow");
        tx.close().await?;

        let tx = driver.transaction(&database, TransactionType::Write).await?;
        tx.query(r#"match $g isa access_grant,has grant_id "team-write";$reviewer isa identity,has identity_id "carol";
          insert $review isa access_review,links(grant:$g,reviewer:$reviewer),has review_id "review-1",has reviewed_at 2026-10-20T00:00:00,has review_outcome "revoked",has reason "Project ended";"#).await?;
        ensure!(!allowed(&tx,"bob","tenant-a","files","write",now).await?, "Revoked review ignored");
        let history=documents(&tx,r#"match $g isa access_grant,has grant_id "team-write";fetch {"id":$g.grant_id};"#).await?;
        ensure!(history.len()==1,"Revocation deleted grant history");
        tx.close().await?;
        Ok(())
    }).catch_unwind().await;
    // Exact generated target, validated even on failure. Never delete business_dev.
    ensure!(
        database
            .strip_prefix("business_backend_test_")
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok()),
        "Invalid cleanup target"
    );
    driver.databases().get(&database).await?.delete().await?;
    match result {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
