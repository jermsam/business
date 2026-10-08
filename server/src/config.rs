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
    if env::var("BILLING_LIVE").as_deref() == Ok("true") {
        validate_live_billing(
            &env::var("TYPEDB_DB").unwrap_or_default(),
            &env::var("PORTAL_ORIGIN_SECRET").unwrap_or_default(),
            &env::var("BILLING_TICK_SECRET").unwrap_or_default(),
            &env::var("STRIPE_WEBHOOK_SECRET").unwrap_or_default(),
            &env::var("STRIPE_SECRET_KEY").unwrap_or_default(),
        )?;
        anyhow::ensure!(
            env::var("BILLING_ENABLED").as_deref() == Ok("true"),
            "Live billing requires its scheduler and webhook ingress"
        );
    }
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

// Live mode is a deliberate deployment step; validation data and unprotected
// origins must never become a live billing system by toggling one flag.
fn validate_live_billing(
    database: &str,
    origin: &str,
    tick: &str,
    webhook: &str,
    stripe: &str,
) -> Result<()> {
    anyhow::ensure!(
        database == "business_prod",
        "Live billing requires the separately provisioned business_prod database"
    );
    anyhow::ensure!(
        origin.len() >= 32 && tick.len() >= 32 && origin != tick,
        "Live billing requires distinct origin and scheduler secrets"
    );
    anyhow::ensure!(
        webhook.starts_with("whsec_") && webhook.len() >= 32,
        "Live billing requires a configured Stripe webhook secret"
    );
    anyhow::ensure!(
        stripe.starts_with("sk_live_") || stripe.starts_with("rk_live_"),
        "Live billing requires a live Stripe key"
    );
    Ok(())
}
#[cfg(test)]
mod billing_config_tests {
    use super::validate_live_billing;
    #[test]
    fn live_billing_rejects_validation_database_and_missing_protections() {
        let origin = "a".repeat(64);
        let tick = "b".repeat(64);
        let webhook = format!("whsec_{}", "c".repeat(32));
        assert!(validate_live_billing(
            "business_prod",
            &origin,
            &tick,
            &webhook,
            "sk_live_fixture"
        )
        .is_ok());
        assert!(
            validate_live_billing("business_dev", &origin, &tick, &webhook, "sk_live_fixture")
                .is_err()
        );
        assert!(
            validate_live_billing("business_prod", "", &tick, &webhook, "sk_live_fixture").is_err()
        );
        assert!(validate_live_billing(
            "business_prod",
            &origin,
            &origin,
            &webhook,
            "sk_live_fixture"
        )
        .is_err());
        assert!(
            validate_live_billing("business_prod", &origin, &tick, "", "sk_live_fixture").is_err()
        );
        assert!(validate_live_billing(
            "business_prod",
            &origin,
            &tick,
            &webhook,
            "sk_test_fixture"
        )
        .is_err());
    }
}
