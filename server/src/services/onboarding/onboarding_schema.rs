use serde::Deserialize;
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Invite { name: String, email: String },
    RevokeInvite { customer_id: String },
    RenewInvite { customer_id: String },
    Accept { token: String, password: String },
}
pub fn email(raw: &str) -> anyhow::Result<String> {
    let value = raw.trim().to_ascii_lowercase();
    anyhow::ensure!(
        value.len() <= 254
            && value.split('@').count() == 2
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b)),
        "Invalid email"
    );
    let (local, host) = value.split_once('@').unwrap();
    anyhow::ensure!(
        !local.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.'),
        "Invalid email"
    );
    Ok(value)
}
