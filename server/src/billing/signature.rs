//! Provider-level authenticity only. Product authorization must still verify stored bindings.
use anyhow::{ensure, Result};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub fn verify_signature(secret: &str, header: &str, body: &[u8], now: i64) -> Result<()> {
    ensure!(!secret.is_empty(), "Webhook not configured");
    let timestamps: Vec<_> = header
        .split(',')
        .filter_map(|s| s.trim().strip_prefix("t="))
        .collect();
    ensure!(timestamps.len() == 1, "Invalid timestamp");
    let timestamp = timestamps[0].parse::<i64>()?;
    ensure!(now.abs_diff(timestamp) <= 300, "Expired webhook");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    mac.update(timestamps[0].as_bytes());
    mac.update(b".");
    mac.update(body);
    for signature in header
        .split(',')
        .filter_map(|s| s.trim().strip_prefix("v1="))
    {
        if signature.len() != 64 || !signature.bytes().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let bytes: Result<Vec<u8>, _> = (0..64)
            .step_by(2)
            .map(|i| u8::from_str_radix(&signature[i..i + 2], 16))
            .collect();
        if let Ok(bytes) = bytes {
            if mac.clone().verify_slice(&bytes).is_ok() {
                return Ok(());
            }
        }
    }
    anyhow::bail!("Invalid signature")
}
