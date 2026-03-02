use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::DogApp;
use dog_typedb::adapter::TypeDBState as TypeDBStateTrait;
use dog_typedb::load_schema_from_file;
use serde_json::Value;
use std::sync::Arc;
use typedb_driver::{Credentials, DriverOptions, TypeDBDriver};

#[derive(Clone)]
pub struct TypeDBState {
    pub driver: Arc<TypeDBDriver>,
    pub database: String,
}

impl TypeDBState {
    pub async fn initialize(app: &DogApp<Value, BusinessParams>) -> Result<()> {
        let addr: String = app.get("typedb.addr").unwrap();
        let database: String = app.get("typedb.db").unwrap();
        let username: String = app.get("typedb.username").unwrap();
        let password: String = app.get("typedb.password").unwrap();
        let tls_str: String = app.get("typedb.tls").unwrap();
        let environment: String = app.get("typedb.environment").unwrap();
        let force_recreate_str: String = app.get("typedb.force_recreate").unwrap();
        let tls: bool = tls_str == "true";
        let force_recreate: bool = force_recreate_str == "true";

        let options = DriverOptions::new(tls, None)?;
        let credentials = Credentials::new(username.as_str(), password.as_str());
        let driver = Arc::new(TypeDBDriver::new(addr, credentials, options).await?);

        let databases = driver.databases().all().await?;
        let database_exists = databases.iter().any(|d| d.name() == database);

        // Only recreate database in development mode or when explicitly forced
        let should_recreate = (environment == "development" || force_recreate) && database_exists;

        if should_recreate {
            println!(
                "Recreating TypeDB database '{}' (environment: {}, force: {})",
                database, environment, force_recreate
            );
            driver.databases().get(&database).await?.delete().await?;
        }

        if !database_exists || should_recreate {
            println!("Creating TypeDB database: {}", database);
            driver.databases().create(&database).await?;

            // Load schema and functions on new database
            let schema_paths = ["schema.tql", "functions.tql"];
            load_schema_from_file(&driver, &database, &schema_paths).await?;
            println!("Successfully loaded schema and functions");
        } else {
            println!("Using existing TypeDB database: {}", database);
        }

        let state = Arc::new(Self { driver, database });
        app.set("typedb", state);
        Ok(())
    }
}
impl TypeDBStateTrait for TypeDBState {
    fn driver(&self) -> &Arc<TypeDBDriver> {
        &self.driver
    }

    fn database(&self) -> &str {
        self.database.as_str()
    }
}
