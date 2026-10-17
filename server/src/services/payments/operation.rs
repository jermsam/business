//! Stable operation identity across independent products; not an authorization credential.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Sandbox,
    Live,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationScope {
    pub merchant_id: String,
    pub product_id: String,
    pub environment: Environment,
    pub operation_id: String,
}
impl OperationScope {
    /// Call only after checking the authenticated application's stored merchant/product grant.
    /// Store a separate canonical request fingerprint: this key must never change with payload.
    pub fn idempotency_key(&self) -> Result<String> {
        for value in [&self.merchant_id, &self.product_id, &self.operation_id] {
            ensure!(
                !value.is_empty()
                    && value.len() <= 128
                    && value
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
                "Invalid payment scope identifier"
            );
        }
        let canonical = serde_json::to_vec(&(
            "payment-v1",
            &self.merchant_id,
            &self.product_id,
            self.environment,
            &self.operation_id,
        ))?;
        Ok(format!("pay-v1-{:x}", Sha256::digest(canonical)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retries_are_stable_but_products_merchants_and_environments_are_isolated() {
        let scope = OperationScope {
            merchant_id: "jitpomi".into(),
            product_id: "invoicing".into(),
            environment: Environment::Sandbox,
            operation_id: "order-1".into(),
        };
        let key = scope.idempotency_key().unwrap();
        assert_eq!(key, scope.idempotency_key().unwrap());
        let mut other = scope.clone();
        other.product_id = "seyfr-shop".into();
        assert_ne!(key, other.idempotency_key().unwrap());
        other = scope.clone();
        other.merchant_id = "other".into();
        assert_ne!(key, other.idempotency_key().unwrap());
        other = scope.clone();
        other.environment = Environment::Live;
        assert_ne!(key, other.idempotency_key().unwrap());
        other = scope;
        other.operation_id = "".into();
        assert!(other.idempotency_key().is_err());
    }
}
