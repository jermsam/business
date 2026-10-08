use serde::Deserialize;
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Request {
        email: String,
    },
    Reset {
        token: String,
        password: String,
        confirm_password: String,
    },
}
pub fn password(value: &str, confirmation: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value == confirmation && (12..=72).contains(&value.len()) && !value.trim().is_empty(),
        "Use matching passwords of 12–72 bytes"
    );
    Ok(())
}
