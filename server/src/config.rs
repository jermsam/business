use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::DogApp;
use serde_json::Value;
use std::env;

pub fn config(app: &DogApp<Value, BusinessParams>) -> Result<()> {
    config_http(app)?;
    config_typedb(app)?;
    configure_auth(app)
}

fn config_http(app: &DogApp<Value, BusinessParams>) -> Result<()> {
    let host = env::var("HTTP_HOST")?;
    let port = env::var("HTTP_PORT")?;
    app.set("http.host", host);
    app.set("http.port", port);
    Ok(())
}

fn config_typedb(app: &DogApp<Value, BusinessParams>) -> Result<()> {
    let addr = env::var("TYPEDB_ADDR")?;
    let db = env::var("TYPEDB_DB")?;
    let username = env::var("TYPEDB_USERNAME")?;
    let password = env::var("TYPEDB_PASSWORD")?;
    let tls = env::var("TYPEDB_TLS").unwrap_or("false".to_string());
    app.set("typedb.addr", addr);
    app.set("typedb.db", db);
    app.set("typedb.username", username);
    app.set("typedb.password", password);
    app.set("typedb.tls", tls);
    Ok(())
}

fn configure_auth(dog_app: &DogApp<Value, BusinessParams>) -> Result<()> {
    let jwt_secret = env::var("AUTH_JWT_SECRET").unwrap_or_else(|_| "dev-secret".to_string());
    let service = env::var("AUTH_SERVICE").unwrap_or_else(|_| "accounts".to_string());
    let entity = env::var("AUTH_ENTITY").unwrap_or_else(|_| "user".to_string());
    let super_company = env::var("SUPER_COMPANY").unwrap_or_else(|_| "JITPOMI".to_string());

    dog_app.set("auth.jwt.secret", jwt_secret);
    dog_app.set("auth.service", service);
    dog_app.set("auth.entity", entity);
    dog_app.set("auth.super_company", super_company);
    Ok(())
}
