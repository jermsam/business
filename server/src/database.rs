//! Canonical TypeQL sources and explicit operator provisioning. No startup migrations.
use anyhow::{ensure, Result};
use futures::TryStreamExt;
use serde_json::Value;
use typedb_driver::{Transaction, TransactionType, TypeDBDriver};
pub const SCHEMA: &str = include_str!("schema.tql");
pub const FUNCTIONS: &str = include_str!("functions.tql");
pub const SCHEMA_POLICY: &str = include_str!("schema-policy.tql");
pub const ACCOUNT_LINKS: &str = include_str!("account_links.tql");
pub const BILLING: &str = include_str!("billing.tql");
pub async fn documents(tx: &Transaction, query: &str) -> Result<Vec<Value>> {
    let mut stream = tx.query(query).await?.into_documents();
    let mut rows = Vec::new();
    while let Some(row) = stream.try_next().await? {
        ensure!(rows.len() < 100_000, "Result limit exceeded");
        rows.push(serde_json::from_str(&row.into_json().to_string())?);
    }
    Ok(rows)
}
pub fn target(name: &str) -> Result<()> {
    ensure!(
        matches!(
            name,
            "business_dev" | "business_native_dev" | "business_prod"
        ) || name
            .strip_prefix("business_backend_test_")
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok()),
        "Use an explicit Business database"
    );
    Ok(())
}
pub async fn verify(driver: &TypeDBDriver, name: &str) -> Result<()> {
    let schema = driver.databases().get(name).await?.schema().await?;
    ensure!(schema.contains("dog.security.scope")&&schema.contains("scope_resource")&&schema.contains("credential_version")&&schema.contains("bootstrap_record"),"Database is not the canonical Business model; provision a new target explicitly. Startup will not migrate or reset it");
    Ok(())
}
pub async fn install(driver: &TypeDBDriver, name: &str) -> Result<()> {
    target(name)?;
    ensure!(
        !driver.databases().contains(name).await?,
        "Database exists; no overwrite or reset performed"
    );
    driver.databases().create(name).await?;
    // On failure leave this exact new target for inspection, never delete a database implicitly.
    let tx = driver.transaction(name, TransactionType::Schema).await?;
    for source in [SCHEMA, FUNCTIONS, SCHEMA_POLICY, BILLING, ACCOUNT_LINKS] {
        tx.query(source).await?;
    }
    tx.commit().await?;
    verify(driver, name).await
}
pub struct Owner {
    pub email: String,
    pub name: String,
    pub tenant_name: String,
    pub password: String,
}
impl Owner {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.email.len() <= 254
                && self
                    .email
                    .split_once('@')
                    .is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.contains('@'))
                && !self
                    .email
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace()),
            "Valid owner email required"
        );
        ensure!(
            [&self.name, &self.tenant_name]
                .iter()
                .all(|s| !s.trim().is_empty()
                    && s.len() <= 120
                    && !s.chars().any(char::is_control)),
            "Owner and tenant names must contain 1–120 characters"
        );
        ensure!(
            (12..=72).contains(&self.password.len()) && !self.password.trim().is_empty(),
            "Owner password must contain 12–72 bytes"
        );
        Ok(())
    }
}
pub async fn bootstrap(
    driver: &TypeDBDriver,
    name: &str,
    owner: Owner,
) -> Result<(String, String)> {
    target(name)?;
    owner.validate()?;
    verify(driver, name).await?;
    let hash =
        tokio::task::spawn_blocking(move || bcrypt::hash(owner.password, bcrypt::DEFAULT_COST))
            .await??;
    let identity = uuid::Uuid::new_v4().to_string();
    let tenant = uuid::Uuid::new_v4().to_string();
    let q = |s: &str| serde_json::to_string(s).unwrap();
    let tx = driver.transaction(name, TransactionType::Write).await?;
    let rows = documents(&tx, "match $i isa identity; fetch {'id':$i.identity_id};").await?;
    ensure!(
        rows.is_empty(),
        "Bootstrap requires an empty identity registry"
    );
    tx.query(format!(r#"insert
       $lock isa bootstrap_record,has bootstrap_id "initial";
       $t isa tenant,has tenant_id "{tenant}",has name {},has status "active",has revision "initial";
       $i isa person,has identity_id "{identity}",has email {},has name {},has password_hash {},has credential_version "initial",has status "active";
       (tenant:$t,member:$i) isa membership,has membership_role "owner",has start_at {};
    "#,q(&owner.tenant_name),q(&owner.email.trim().to_ascii_lowercase()),q(&owner.name),q(&hash),chrono::Utc::now().naive_utc().format("%Y-%m-%dT%H:%M:%S%.6f"))).await?;
    tx.commit().await?;
    Ok((tenant, identity))
}
#[cfg(test)]
#[path = "../tests/unit/database/tests.rs"]
mod tests;
