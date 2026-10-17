use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn token(id: &str, version: &str) -> anyhow::Result<String> {
    let secret = std::env::var("AUTH_JWT_SECRET")?;
    anyhow::ensure!(secret.len() >= 32, "Recovery signing secret unavailable");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    mac.update(b"jitpomi-password-recovery-v1\0");
    mac.update(id.as_bytes());
    mac.update(b"\0");
    mac.update(version.as_bytes());
    Ok(mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
