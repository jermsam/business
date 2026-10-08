//! Explicit operator commands. Never replaces or deletes a database, or activates payments.
use crate::{
    access::documents,
    services::{billing::billing_schema::quoted, recovery::RecoveryService},
    typedb::TypeDBState,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, sync::Arc};
use typedb_driver::{TransactionType, TypeDBDriver};

fn digest_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut block = [0; 65536];
    loop {
        let n = file.read(&mut block)?;
        if n == 0 {
            break;
        }
        hash.update(&block[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        anyhow::bail!("Private backup permissions require Unix");
    }
    Ok(())
}
fn private_file(path: &Path) -> Result<()> {
    use std::fs::OpenOptions;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
    }
    Ok(())
}
pub async fn backup(driver: &TypeDBDriver, db: &str, path: &Path) -> Result<()> {
    private_dir(path).context("Backup destination must be new, under a trusted private parent")?;
    let schema = path.join("schema.tql");
    let data = path.join("data.typedb");

    driver
        .databases()
        .get(db)
        .await?
        .export_to_file(&schema, &data)
        .await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in [&schema, &data] {
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))?;
        }
    }

    let manifest = json!({"format":1,"database":db,"created_at":chrono::Utc::now().to_rfc3339(),"schema_sha256":digest_file(&schema)?,"data_sha256":digest_file(&data)?});
    let target = path.join("manifest.json");
    private_file(&target)?;
    std::fs::write(target, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}
pub async fn restore(driver: &TypeDBDriver, db: &str, path: &Path) -> Result<()> {
    ensure!(
        !driver.databases().contains(db).await?,
        "Restore target already exists; no changes made"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path.join("manifest.json"))?)?;
    ensure!(manifest["format"] == 1, "Unsupported backup format");
    for (name, key) in [
        ("schema.tql", "schema_sha256"),
        ("data.typedb", "data_sha256"),
    ] {
        ensure!(
            manifest[key] == digest_file(&path.join(name))?,
            "Backup checksum mismatch"
        );
    }
    driver
        .databases()
        .import_from_file(
            db,
            std::fs::read_to_string(path.join("schema.tql"))?,
            path.join("data.typedb"),
        )
        .await?;
    // An import failure leaves the new target for inspection; this tool never deletes data.
    let tx = driver.transaction(db, TransactionType::Read).await?;
    documents(&tx, "match $u isa user; fetch {\"id\":[$u.biz_id]};").await?;
    tx.close().await?;
    Ok(())
}
pub(crate) async fn bootstrap(
    state: Arc<TypeDBState>,
    email: &str,
    domain: &str,
) -> Result<String> {
    let email = crate::services::onboarding::onboarding_schema::email(email)?;
    ensure!(
        !domain.is_empty()
            && domain.len() <= 253
            && domain
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'),
        "Use a lowercase workspace domain"
    );
    let user = uuid::Uuid::new_v4();
    let tenant = uuid::Uuid::new_v4().to_string();
    let revision = uuid::Uuid::new_v4();
    // No shared/default password exists. Owner must use the expiring recovery flow.
    let hash = bcrypt::hash(uuid::Uuid::new_v4().to_string(), 12)?;
    let query = format!(
        r#"match not {{$existing isa user;}};not {{$existing_tenant isa company;}};
 insert $gate isa portal_bootstrap,has auth_request_key "initial-owner";
 $u isa user,has biz_id "{user}",has email {},has password {},has biz_state "active";
 $t isa company,has biz_id "{tenant}",has company_domain {},has biz_state "active",has biz_revision "{revision}";
 (tenant:$t,member:$u) isa tenant_membership,has biz_migrated true;
 (tenant:$t,person:$u) isa biz_membership,has biz_id "{tenant}:{user}",has biz_role "owner",has biz_state "active",has biz_start 1970-01-01T00:00:00;
 fetch {{"merchant_id":$t.biz_id}};"#,
        quoted(&email),
        quoted(&hash),
        quoted(domain)
    );
    crate::access::write_atomic_one(&state, &query).await?;
    RecoveryService::new(state).request(&email).await?;
    Ok(tenant)
}
pub async fn run() -> Result<()> {
    if let Ok(path) = std::env::var("BUSINESS_ENV_FILE") {
        dotenvy::from_path(path)?;
    }
    let db = std::env::var("TYPEDB_DB")?;
    ensure!(
        matches!(db.as_str(), "business_dev" | "business_prod")
            || db
                .strip_prefix("business_restore_")
                .is_some_and(|v| uuid::Uuid::parse_str(v).is_ok()),
        "Use a dedicated Business database"
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
    let state = Arc::new(TypeDBState {
        driver: driver.clone(),
        database: db.clone(),
    });
    match std::env::args().nth(1).as_deref(){
  Some("backup")=>{backup(&driver,&db,Path::new(&std::env::var("BUSINESS_BACKUP_DIR")?)).await?;println!("Private backup and checksum manifest complete");},
  Some("restore-new")=>{restore(&driver,&db,Path::new(&std::env::var("BUSINESS_BACKUP_DIR")?)).await?;println!("Restored into new database; validate policies before switching runtime");},
  Some("bootstrap")=>{let id=bootstrap(state,&std::env::var("BUSINESS_OWNER_EMAIL")?,&std::env::var("BUSINESS_WORKSPACE_DOMAIN")?).await?;println!("Merchant ID: {id}. Recovery queued; configure BILLING_MERCHANT_ID and deliver recovery before sign-in.");},
  Some("recovery-request")=>{RecoveryService::new(state).request(&std::env::var("BUSINESS_OWNER_EMAIL")?).await?;println!("Recovery request queued");},
  Some("recovery-tick")=>{let(sent,errors)=RecoveryService::new(state).process_pending().await?;println!("Recovery sent={sent} errors={errors}");ensure!(errors==0,"Recovery delivery needs inspection");},
  Some("preflight")=>{
   crate::build().await?;
   ensure!(std::env::var("PORTAL_ORIGIN_SECRET").unwrap_or_default().len()>=32,"Portal origin protection required");
   ensure!(std::env::var("BILLING_MAIL_ENABLED").as_deref()==Ok("true"),"Recovery and billing mail must be enabled");
   ensure!(!std::env::var("RESEND_API_KEY").unwrap_or_default().is_empty(),"Email key required");
   let id=uuid::Uuid::parse_str(&std::env::var("BILLING_MERCHANT_ID")?)?;
   let tx=driver.transaction(&db,TransactionType::Read).await?;
   let query=format!(r#"match $t isa company,has biz_id "{id}";$u isa user;let $owner=biz_owner($u,$t,{});$owner==true;fetch {{"id":$u.biz_id}};"#,chrono::Utc::now().naive_utc().format("%Y-%m-%dT%H:%M:%S%.6f"));
   ensure!(!documents(&tx,&query).await?.is_empty(),"Configured merchant needs an active owner");
   documents(&tx,"match $r isa portal_recovery;fetch {\"id\":$r.biz_id};").await?;tx.close().await?;
   println!("Runtime configuration, database connection, recovery schema and merchant owner verified; no payment attempted");
  },
  Some("attention")=>{
   let tx=driver.transaction(&db,TransactionType::Read).await?;
   for (label,query) in [
    ("recovery",r#"match $r isa portal_recovery,has auth_status $s;{$s=="sending";} or {$s=="confirming";} or {$s=="attention";};fetch {"id":$r.biz_id,"status":$s};"#),
    ("notices",r#"match $r isa bill_notice,has bill_status $s;{$s=="sending";} or {$s=="attention";};fetch {"id":$r.bill_key,"status":$s};"#),
    ("invoices",r#"match $r isa bill_invoice,has bill_status $s;{$s=="issuing";} or {$s=="attention";};fetch {"id":$r.bill_key,"status":$s};"#)
   ] {println!("{label}: {}",serde_json::to_string(&documents(&tx,query).await?)?);}
   tx.close().await?;
  },
  _=>anyhow::bail!("Use backup, restore-new, bootstrap, recovery-request, recovery-tick, preflight, attention")
 }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Creates and removes isolated validation databases on the existing Cloud cluster"]
    async fn hosted_bootstrap_backup_restore_operations() {
        let (app, _) = crate::build().await.unwrap();
        let source = app.get::<Arc<TypeDBState>>("typedb").unwrap();
        assert_eq!(source.database, "business_dev");
        let d = &source.driver;
        let db = format!("business_restore_{}", uuid::Uuid::new_v4().simple());
        let target = format!("business_restore_{}", uuid::Uuid::new_v4().simple());
        d.databases().create(&db).await.unwrap();
        let result=async {
   let schema=d.databases().get("business_dev").await?.schema().await?;
   let tx=d.transaction(&db,TransactionType::Schema).await?;tx.query(schema).await?;tx.commit().await?;
   let state=Arc::new(TypeDBState{driver:d.clone(),database:db.clone()});
   let (first,second)=tokio::join!(bootstrap(state.clone(),"bootstrap-validation@example.com","bootstrap.example.com"),bootstrap(state.clone(),"concurrent-validation@example.com","concurrent.example.com"));
   ensure!(usize::from(first.is_ok())+usize::from(second.is_ok())==1,"Exactly one initial owner may be created");let merchant=first.or(second)?;
   ensure!(bootstrap(state,"second@example.com","second.example.com").await.is_err(),"Bootstrap must not add another owner");
   let tx=d.transaction(&db,TransactionType::Read).await?;
   let rows=documents(&tx,"match $m isa biz_membership,has biz_role \"owner\",links (tenant:$t,person:$u);let $ok=biz_owner($u,$t,2026-10-08T00:00:00);fetch {\"tenant\":$t.biz_id,\"allowed\":$ok};").await?;
   ensure!(rows.len()==1&&rows[0]["tenant"]==merchant&&rows[0]["allowed"]==true,"Bootstrap owner policy failed");tx.close().await?;
   let tmp=tempfile::tempdir()?;let folder=tmp.path().join("backup");backup(d,&db,&folder).await?;
   ensure!(backup(d,&db,&folder).await.is_err(),"Backup overwrite must fail");
   let manifest=std::fs::read(folder.join("manifest.json"))?;std::fs::write(folder.join("manifest.json"),b"{\"format\":1}")?;
   ensure!(restore(d,&target,&folder).await.is_err(),"Corrupt manifest must fail");ensure!(!d.databases().contains(&target).await?,"No target on checksum failure");std::fs::write(folder.join("manifest.json"),manifest)?;
   restore(d,&target,&folder).await?;ensure!(restore(d,&target,&folder).await.is_err(),"Existing restore target must fail");
   let tx=d.transaction(&target,TransactionType::Read).await?;let rows=documents(&tx,"match $r isa portal_recovery;fetch {\"status\":$r.auth_status};").await?;ensure!(rows.len()==1&&rows[0]["status"]=="queued","Recovery request survives restore");tx.close().await?;
   Ok::<_,anyhow::Error>(())
  }.await;
        for name in [&db, &target] {
            if d.databases().contains(name).await.unwrap() {
                d.databases()
                    .get(name)
                    .await
                    .unwrap()
                    .delete()
                    .await
                    .unwrap();
            }
        }
        result.unwrap();
        println!("OPERATIONS bootstrap, owner policy, private backup, integrity check and non-destructive restore passed");
    }
}
