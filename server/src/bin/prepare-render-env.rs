//! Prepare an owner-only environment import file without printing credentials.
use anyhow::{ensure, Context, Result};
use std::{collections::HashMap, io::Write};
fn main() -> Result<()> {
    let source = std::env::var("BUSINESS_ENV_FILE").context("BUSINESS_ENV_FILE required")?;
    let output = std::env::args().nth(1).context("Output path required")?;
    let vars: HashMap<String, String> =
        dotenvy::from_path_iter(source)?.collect::<std::result::Result<_, _>>()?;
    ensure!(
        vars.get("TYPEDB_DB").map(String::as_str) == Some("business_dev"),
        "Validation database required"
    );
    ensure!(
        vars.get("TYPEDB_USERNAME").map(String::as_str) != Some("admin"),
        "Non-admin credentials required"
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&output)?;
    for key in ["TYPEDB_ADDR", "TYPEDB_USERNAME", "TYPEDB_PASSWORD"] {
        let value = vars.get(key).with_context(|| format!("Missing {key}"))?;
        ensure!(!value.is_empty(), "Empty {key}");
        writeln!(file, "{key}={}", serde_json::to_string(value)?)?;
    }
    writeln!(file,"HTTP_HOST=0.0.0.0\nHTTP_PORT=10000\nENVIRONMENT=production\nTYPEDB_DB=business_dev\nTYPEDB_TLS=true\nTYPEDB_FORCE_RECREATE=false")?;
    writeln!(
        file,
        "AUTH_JWT_SECRET={}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )?;
    file.sync_all()?;
    println!("Prepared private validation environment file: {output}");
    Ok(())
}
