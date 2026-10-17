//! Shared Stripe transport. Callers must persist an authorized operation before side effects.
use anyhow::{ensure, Result};
use reqwest::{Client, Method};
use serde_json::Value;

#[derive(Clone)]
pub struct StripeTransport {
    http: Client,
    live: bool,
}
impl StripeTransport {
    pub fn require_environment(&self, live: bool) -> Result<()> {
        ensure!(self.live == live, "Payment transport environment mismatch");
        Ok(())
    }
    pub fn new(http: Client, live: bool) -> Self {
        Self { http, live }
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        key: &str,
        form: &[(String, String)],
    ) -> Result<Value> {
        ensure!(
            path.starts_with('/') && !path.contains(['?', '#', '\r', '\n']),
            "Invalid provider path"
        );
        let token = std::env::var("STRIPE_SECRET_KEY")?;
        validate_mode(&token, self.live)?;
        let mut url = reqwest::Url::parse(&format!("https://api.stripe.com/v1{path}"))?;
        if method == Method::GET && !form.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(form.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        }
        let mut request = self
            .http
            .request(method.clone(), url)
            .bearer_auth(token)
            .header("Stripe-Version", "2026-03-25.dahlia");
        if method != Method::GET {
            request = request.header("Idempotency-Key", key).form(form);
        }
        let response = request.send().await?;
        ensure!(
            response.status().is_success(),
            "Stripe request failed with HTTP {}; reconcile before retrying",
            response.status()
        );
        let result: Value = response.json().await?;
        if let Some(mode) = result["livemode"].as_bool() {
            ensure!(mode == self.live, "Provider mode mismatch");
        }
        Ok(result)
    }
}
fn validate_mode(token: &str, live: bool) -> Result<()> {
    ensure!(
        if live {
            token.starts_with("sk_live_") || token.starts_with("rk_live_")
        } else {
            token.starts_with("sk_test_") || token.starts_with("rk_test_")
        },
        "Stripe key does not match billing mode"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_cannot_cross_environments() {
        for key in ["sk_test_fixture", "rk_test_fixture"] {
            assert!(validate_mode(key, false).is_ok());
            assert!(validate_mode(key, true).is_err());
        }
        for key in ["sk_live_fixture", "rk_live_fixture"] {
            assert!(validate_mode(key, true).is_ok());
            assert!(validate_mode(key, false).is_err());
        }
        assert!(validate_mode("", false).is_err());
    }
}
