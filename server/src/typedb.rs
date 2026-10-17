use crate::services::BusinessParams;
use anyhow::{Context, Result};
use dog_core::DogAppBuilder;
use dog_typedb::{adapter::TypeDBState as State, TypeDBDriverFactory};
use serde_json::Value;
use std::sync::Arc;
use typedb_driver::TypeDBDriver;
#[derive(Clone)]
pub struct TypeDBState {
    pub driver: Arc<TypeDBDriver>,
    pub database: String,
}
impl TypeDBState {
    pub async fn initialize(app: &mut DogAppBuilder<Value, BusinessParams>) -> Result<()> {
        let addr: String = app.get("typedb.addr").context("Missing TypeDB address")?;
        let database: String = app.get("typedb.db").context("Missing database")?;
        let username: String = app
            .get("typedb.username")
            .context("Missing database username")?;
        let password: String = app
            .get("typedb.password")
            .context("Missing database password")?;
        let driver =
            Arc::new(TypeDBDriverFactory::connect(&addr, &username, &password, true).await?);
        anyhow::ensure!(
            driver.databases().contains(&database).await?,
            "Database does not exist; provision it explicitly before starting the server"
        );
        app.set("typedb", Arc::new(Self { driver, database }));
        Ok(())
    }
}
impl State for TypeDBState {
    fn driver(&self) -> &Arc<TypeDBDriver> {
        &self.driver
    }
    fn database(&self) -> &str {
        &self.database
    }
}
