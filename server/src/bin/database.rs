//! Explicit database administration; no implicit migration or destructive reset.
use anyhow::{ensure, Result};
#[tokio::main]
async fn main() -> Result<()> {
    if let Ok(path) = std::env::var("BUSINESS_ENV_FILE") {
        dotenvy::from_path(path)?;
    }
    let command = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Use list, create, verify, bootstrap or create-user"))?;
    ensure!(
        ["list", "create", "verify", "bootstrap", "create-user"].contains(&command.as_str()),
        "Unknown database command; no connection or changes made"
    );
    let driver = dog_typedb::TypeDBDriverFactory::connect(
        &std::env::var("TYPEDB_ADDR")?,
        &std::env::var("TYPEDB_USERNAME")?,
        &std::env::var("TYPEDB_PASSWORD")?,
        true,
    )
    .await?;
    match command.as_str() {
        "list" => {
            for database in driver.databases().all().await? {
                println!("{}", database.name());
            }
        }
        "create-user" => {
            let user = std::env::var("BUSINESS_NEW_USER")?;
            ensure!(
                matches!(user.as_str(), "business_dev_app" | "business_prod_app"),
                "Use a Business runtime user"
            );
            ensure!(
                !driver.users().contains(&user).await?,
                "User exists; not modified"
            );
            let password = std::env::var("BUSINESS_NEW_PASSWORD")?;
            ensure!(
                password.len() >= 16,
                "Runtime password must contain at least 16 bytes"
            );
            driver.users().create(user, password).await?;
            println!("Created runtime account; no credential is printed");
        }
        _ => {
            let database = std::env::var("TYPEDB_DB")?;
            server::database::target(&database)?;
            match command.as_str() {
                "create" => {
                    server::database::install(&driver, &database).await?;
                    println!("Provisioned canonical model; no existing database was changed");
                }
                "verify" => {
                    server::database::verify(&driver, &database).await?;
                    println!("Canonical model verified");
                }
                "bootstrap" => {
                    let owner = server::database::Owner {
                        email: std::env::var("BUSINESS_OWNER_EMAIL")?,
                        name: std::env::var("BUSINESS_OWNER_NAME")?,
                        tenant_name: std::env::var("BUSINESS_TENANT_NAME")?,
                        password: std::env::var("BUSINESS_OWNER_PASSWORD")?,
                    };
                    let (tenant, identity) =
                        server::database::bootstrap(&driver, &database, owner).await?;
                    println!("Initial owner created: tenant={tenant} identity={identity}; no password or token is printed");
                }
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}
