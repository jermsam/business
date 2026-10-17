//! Immutable, product-independent requests. No invoice, card, or delivery fields.
use super::operation::OperationScope;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentRequest {
    pub scope: OperationScope,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: u8,
    pub reference: String,
}
impl PaymentRequest {
    pub fn validate(&self) -> Result<()> {
        self.scope.idempotency_key()?;
        ensure!(self.amount_minor > 0, "Payment amount must be positive");
        ensure!(
            self.currency.len() == 3 && self.currency.bytes().all(|c| c.is_ascii_uppercase()),
            "Invalid currency code"
        );
        ensure!(self.currency_exponent <= 3, "Unsupported currency exponent");
        ensure!(
            !self.reference.is_empty()
                && self.reference.len() <= 128
                && self
                    .reference
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
            "Invalid product reference"
        );
        // Syntax is not provider support: checkout adapters must validate currency and exponent.
        Ok(())
    }
    pub fn fingerprint(&self) -> Result<String> {
        self.validate()?;
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(self)?)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::payments::operation::Environment;
    pub fn fixture() -> PaymentRequest {
        PaymentRequest {
            scope: OperationScope {
                merchant_id: "merchant".into(),
                product_id: "shop".into(),
                environment: Environment::Sandbox,
                operation_id: "order-1".into(),
            },
            amount_minor: 1200,
            currency: "USD".into(),
            currency_exponent: 2,
            reference: "cart-1".into(),
        }
    }
    #[test]
    fn changed_payload_retains_key_but_changes_fingerprint() {
        let request = fixture();
        let mut other = request.clone();
        other.amount_minor += 1;
        assert_eq!(
            request.scope.idempotency_key().unwrap(),
            other.scope.idempotency_key().unwrap()
        );
        assert_ne!(request.fingerprint().unwrap(), other.fingerprint().unwrap());
    }
    #[test]
    fn rejects_unsafe_or_ambiguous_requests() {
        let mut request = fixture();
        request.amount_minor = 0;
        assert!(request.validate().is_err());
        request = fixture();
        request.currency = "usd".into();
        assert!(request.validate().is_err());
        request = fixture();
        request.reference = "\"; delete $x;".into();
        assert!(request.validate().is_err());
        let value = serde_json::json!({"scope":fixture().scope,"amount_minor":100,"currency":"USD","currency_exponent":2,"reference":"cart","card_number":"private"});
        assert!(serde_json::from_value::<PaymentRequest>(value).is_err());
    }
}
