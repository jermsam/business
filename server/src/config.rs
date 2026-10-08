use crate::services::BusinessParams;
use anyhow::Result;
use dog_core::DogAppBuilder;
use serde_json::Value;
use std::env;

pub fn config(app: &mut DogAppBuilder<Value, BusinessParams>) -> Result<()> {
    validate_security(
        &env::var("AUTH_JWT_SECRET").unwrap_or_default(),
        &env::var("TYPEDB_TLS").unwrap_or_else(|_| "true".into()),
        &env::var("TYPEDB_FORCE_RECREATE").unwrap_or_else(|_| "false".into()),
        &env::var("TYPEDB_USERNAME").unwrap_or_default(),
    )?;
    crate::admission::configured_limit()?;
    config_http(app)?;
    config_typedb(app)?;
    configure_auth(app)
}

fn config_http(app: &mut DogAppBuilder<Value, BusinessParams>) -> Result<()> {
    let host = env::var("HTTP_HOST")?;
    let port = env::var("HTTP_PORT")?;
    app.set("http.host", host);
    app.set("http.port", port);
    Ok(())
}

fn config_typedb(app: &mut DogAppBuilder<Value, BusinessParams>) -> Result<()> {
    let addr = env::var("TYPEDB_ADDR")?;
    let db = env::var("TYPEDB_DB")?;
    let username = env::var("TYPEDB_USERNAME")?;
    anyhow::ensure!(
        username != "admin",
        "The application must use a non-admin TypeDB account"
    );
    let password = env::var("TYPEDB_PASSWORD")?;
    let tls = env::var("TYPEDB_TLS").unwrap_or("true".to_string());
    anyhow::ensure!(tls == "true", "TLS is required in all environments");
    let environment = env::var("ENVIRONMENT").unwrap_or("development".to_string());
    let force_recreate = env::var("TYPEDB_FORCE_RECREATE").unwrap_or("false".to_string());

    anyhow::ensure!(
        force_recreate == "false",
        "Database reset during startup is forbidden"
    );
    app.set("typedb.addr", addr);
    app.set("typedb.db", db);
    app.set("typedb.username", username);
    app.set("typedb.password", password);
    app.set("typedb.tls", tls);
    app.set("typedb.environment", environment);
    app.set("typedb.force_recreate", force_recreate);
    Ok(())
}

fn configure_auth(dog_app: &mut DogAppBuilder<Value, BusinessParams>) -> Result<()> {
    let jwt_secret = env::var("AUTH_JWT_SECRET").unwrap_or_else(|_| String::new());
    anyhow::ensure!(
        jwt_secret.len() >= 32,
        "AUTH_JWT_SECRET must contain at least 32 bytes"
    );
    let service = env::var("AUTH_SERVICE").unwrap_or_else(|_| "accounts".to_string());
    let entity = env::var("AUTH_ENTITY").unwrap_or_else(|_| "user".to_string());
    let super_company_domain =
        env::var("SUPER_COMPANY_DOMAIN").unwrap_or_else(|_| "jitpomi.com".to_string());

    dog_app.set("auth.jwt.secret", jwt_secret);
    dog_app.set("auth.service", service);
    dog_app.set("auth.entity", entity);
    dog_app.set("auth.super_company_domain", super_company_domain);
    Ok(())
}

fn validate_security(secret: &str, tls: &str, reset: &str, user: &str) -> Result<()> {
    anyhow::ensure!(
        secret.len() >= 32 && secret.trim().len() >= 32,
        "A nonblank JWT secret of at least 32 bytes is required"
    );
    anyhow::ensure!(tls == "true", "TLS is required");
    anyhow::ensure!(reset == "false", "Startup reset is forbidden");
    anyhow::ensure!(
        !user.is_empty() && user != "admin",
        "A non-admin runtime account is required"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_insecure_startup() {
        let secret = "test-only-secret-with-more-than-32-bytes";
        assert!(validate_security(secret, "true", "false", "business_dev_app").is_ok());
        assert!(validate_security("dev-secret", "true", "false", "business_dev_app").is_err());
        assert!(validate_security(secret, "false", "false", "business_dev_app").is_err());
        assert!(validate_security(secret, "true", "true", "business_dev_app").is_err());
        assert!(validate_security(secret, "true", "false", "admin").is_err());
    }
}
