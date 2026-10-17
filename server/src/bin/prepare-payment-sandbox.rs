//! Generate owner-only sandbox configuration; never prints credentials.
use anyhow::{ensure, Result};
use sha2::{Digest, Sha256};
use std::io::Write;
fn main() -> Result<()> {
    dotenvy::from_path(std::env::var("BILLING_TEST_ENV")?)?;
    ensure!(
        std::env::var("BILLING_LIVE").as_deref() != Ok("true"),
        "Sandbox only"
    );
    let key = std::env::var("STRIPE_SECRET_KEY")?;
    ensure!(
        key.starts_with("sk_test_") || key.starts_with("rk_test_"),
        "Test Stripe key required"
    );
    let merchant = uuid::Uuid::parse_str(&std::env::var("BILLING_MERCHANT_ID")?)?.to_string();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let apps = serde_json::json!([{"id":"owner-sandbox","token_sha256":format!("{:x}",Sha256::digest(token.as_bytes())),"merchant":merchant,"product":"sandbox-demo","success_url":"https://jitpomi.com/account/payments/?return=success","cancel_url":"https://jitpomi.com/account/payments/?return=cancel","delivery":{"url":"https://jitpomi-business-validation.onrender.com/payments/sandbox/receiver","secret":secret}}]);
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Private output path required"))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    writeln!(file,"PAYMENTS_SANDBOX_ENABLED=true\nPAYMENTS_BROWSER_APP=owner-sandbox\nPAYMENTS_APPLICATIONS_JSON='{}'",apps)?;
    file.sync_all()?;
    println!("Prepared private sandbox application configuration");
    Ok(())
}
