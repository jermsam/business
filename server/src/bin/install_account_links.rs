//! Explicit additive schema installation, restricted to the current sandbox database.
use anyhow::{ensure, Result};
use typedb_driver::TransactionType;
#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::from_path(std::env::var("BUSINESS_ENV_FILE")?)?;
    ensure!(
        std::env::var("TYPEDB_DB")? == "business_dev",
        "Sandbox operator configuration required"
    );
    let driver = dog_typedb::TypeDBDriverFactory::connect(
        &std::env::var("TYPEDB_ADDR")?,
        &std::env::var("TYPEDB_USERNAME")?,
        &std::env::var("TYPEDB_PASSWORD")?,
        true,
    )
    .await?;
    let target = "business_native_dev";
    server::database::verify(&driver, target).await?;
    let schema = driver.databases().get(target).await?.schema().await?;
    if schema.contains("entity account_link") {
        println!("Account-link schema already installed; no change.");
        return Ok(());
    }
    let tx = driver.transaction(target, TransactionType::Schema).await?;
    tx.query(server::database::ACCOUNT_LINKS).await?;
    tx.commit().await?;
    println!("Installed account-link schema in business_native_dev; existing data unchanged.");
    Ok(())
}
