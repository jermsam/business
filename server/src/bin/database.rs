//! Explicit database inspection/provisioning; never deletes a database.
use anyhow::Result;
#[tokio::main]
async fn main() -> Result<()> {
    if let Ok(path) = std::env::var("BUSINESS_ENV_FILE") {
        dotenvy::from_path(path)?;
    }
    let driver = dog_typedb::TypeDBDriverFactory::connect(
        &std::env::var("TYPEDB_ADDR")?,
        &std::env::var("TYPEDB_USERNAME")?,
        &std::env::var("TYPEDB_PASSWORD")?,
        true,
    )
    .await?;
    if std::env::args().nth(1).as_deref() == Some("list") {
        for db in driver.databases().all().await? {
            println!("{}", db.name());
        }
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("create-user") {
        let user = std::env::var("BUSINESS_NEW_USER")?;
        anyhow::ensure!(
            matches!(user.as_str(), "business_dev_app" | "business_prod_app"),
            "Only Business runtime users can be created"
        );
        anyhow::ensure!(
            !driver.users().contains(&user).await?,
            "User already exists; no changes made"
        );
        driver
            .users()
            .create(user, std::env::var("BUSINESS_NEW_PASSWORD")?)
            .await?;
        println!("Created non-admin runtime user");
        return Ok(());
    }
    anyhow::ensure!(
        matches!(
            std::env::args().nth(1).as_deref(),
            Some("create" | "schema" | "migrate-records" | "migrate-access" | "migrate-billing")
        ),
        "Use list, create, schema, migrate-records, migrate-access, or create-user"
    );
    let db = std::env::var("TYPEDB_DB")?;
    anyhow::ensure!(
        matches!(db.as_str(), "business_dev" | "business_prod"),
        "Only dedicated Business databases may be provisioned"
    );
    if std::env::args().nth(1).as_deref() == Some("migrate-billing") {
        anyhow::ensure!(
            db == "business_dev",
            "Validate billing migration in business_dev first"
        );
        apply_schema_sources(
            &driver,
            &db,
            &[
                include_str!("../../migrations/005-billing.tql"),
                include_str!("../../migrations/006-customer-onboarding.tql"),
            ],
            false,
        )
        .await?;
        println!("Applied billing schema to {db}");
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("migrate-access") {
        anyhow::ensure!(
            db == "business_dev",
            "Validate access migration in business_dev first"
        );
        apply_access_migration(&driver, &db).await?;
        println!("Applied workspace access schema/functions to {db}");
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("create") {
        anyhow::ensure!(
            !driver.databases().contains(&db).await?,
            "Database already exists; no changes made"
        );
        driver.databases().create(&db).await?;
    }
    if std::env::args().nth(1).as_deref() == Some("migrate-records") {
        dog_typedb::load_schema_from_file(
            &driver,
            &db,
            &[concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/001-private-records.tql"
            )],
        )
        .await?;
        println!("Applied private-record schema to {db}");
        return Ok(());
    }
    dog_typedb::load_schema_from_file(
        &driver,
        &db,
        &[
            concat!(env!("CARGO_MANIFEST_DIR"), "/schema.tql"),
            concat!(env!("CARGO_MANIFEST_DIR"), "/functions.tql"),
        ],
    )
    .await?;
    apply_access_migration(&driver, &db).await?;
    println!("Provisioned {db}");
    Ok(())
}

/// All access types and function updates commit together; failures close the transaction.
async fn apply_access_migration(driver: &typedb_driver::TypeDBDriver, db: &str) -> Result<()> {
    let sources = [
        include_str!("../../migrations/002-workspace-access.tql"),
        include_str!("../../workspace-functions.tql"),
        include_str!("../../migrations/003-account-lifecycle.tql"),
        include_str!("../../migrations/004-identity-concurrency.tql"),
    ];
    apply_schema_sources(driver, db, &sources, true).await
}
async fn apply_schema_sources(
    driver: &typedb_driver::TypeDBDriver,
    db: &str,
    sources: &[&str],
    backfill: bool,
) -> Result<()> {
    let existing = driver.databases().get(db).await?.schema().await?;
    let names = regex::Regex::new(r"(?m)\bfun\s+([A-Za-z_][A-Za-z_0-9]*)\s*\(")?;
    let known: std::collections::HashSet<_> = names
        .captures_iter(&existing)
        .map(|c| c[1].to_owned())
        .collect();
    let tx = driver
        .transaction_with_options(
            db,
            typedb_driver::TransactionType::Schema,
            typedb_driver::TransactionOptions::new()
                .transaction_timeout(std::time::Duration::from_secs(120)),
        )
        .await?;
    for source in sources {
        let positions: Vec<_> = names
            .captures_iter(source)
            .map(|c| (c.get(0).unwrap().start(), c[1].to_owned()))
            .collect();
        let prefix = &source[..positions.first().map(|p| p.0).unwrap_or(source.len())];
        if prefix
            .lines()
            .any(|l| !l.trim().is_empty() && l.trim() != "define" && !l.trim().starts_with('#'))
        {
            tx.query(prefix).await?;
        }
        for (i, (start, name)) in positions.iter().enumerate() {
            let end = positions.get(i + 1).map(|p| p.0).unwrap_or(source.len());
            let verb = if known.contains(name) {
                "redefine"
            } else {
                "define"
            };
            tx.query(format!("{verb}\n{}", &source[*start..end]))
                .await?;
        }
    }
    if backfill {
        server::access::backfill(&tx).await?;
    }
    tx.commit().await.map_err(|e| {
        anyhow::anyhow!("Access migration commit failed; inspect schema before retrying: {e}")
    })?;
    Ok(())
}
