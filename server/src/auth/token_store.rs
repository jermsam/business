//! Shared durable token revocation. A key constraint arbitrates concurrent refresh consumption.
use anyhow::Result;
use dog_auth::core::TokenStore;
use dog_typedb::TypeDBAdapter;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
pub struct TypeDbTokenStore {
    adapter: TypeDBAdapter,
}
impl TypeDbTokenStore {
    pub fn new(state: Arc<crate::typedb::TypeDBState>) -> Self {
        Self {
            adapter: TypeDBAdapter::new(state),
        }
    }
    fn key(issuer: &str, jti: &str) -> String {
        let mut hash = Sha256::new();
        hash.update((issuer.len() as u64).to_be_bytes());
        hash.update(issuer);
        hash.update(jti);
        format!("{:x}", hash.finalize())
    }
    async fn exists(&self, key: &str) -> Result<bool> {
        #[cfg(test)]
        let started = std::time::Instant::now();
        #[cfg(test)]
        crate::access::trace_enter("revocation-start");
        let out=self.adapter.read(json!({"query":format!(r#"match $t isa auth_revocation, has token_key "{key}"; fetch {{ "key": $t.token_key }};"#)})).await?;
        #[cfg(test)]
        crate::access::trace_stage("revocation-read", started);
        let rows = out
            .pointer("/ok/answers")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid token lookup response"))?;
        Ok(!rows.is_empty())
    }
    async fn consume(&self, issuer: &str, jti: &str, expires: i64) -> Result<bool> {
        if expires <= chrono::Utc::now().timestamp() {
            return Ok(false);
        }
        let key = Self::key(issuer, jti);
        if self.exists(&key).await? {
            return Ok(false);
        }
        let result=self.adapter.write(json!({"query":format!(r#"insert $t isa auth_revocation, has token_key "{key}";"#)})).await;
        match result {
            Ok(_) => Ok(true),
            Err(error) => {
                if self.exists(&key).await? {
                    Ok(false)
                } else {
                    Err(error)
                }
            }
        }
    }
}
#[async_trait::async_trait]
impl TokenStore for TypeDbTokenStore {
    async fn is_revoked(&self, issuer: &str, jti: &str) -> Result<bool> {
        self.exists(&Self::key(issuer, jti)).await
    }
    async fn revoke(&self, issuer: &str, jti: &str, expires_at: i64) -> Result<()> {
        self.consume(issuer, jti, expires_at).await?;
        Ok(())
    }
    async fn consume_refresh(&self, issuer: &str, jti: &str, expires_at: i64) -> Result<bool> {
        self.consume(issuer, jti, expires_at).await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn issuer_and_token_are_unambiguous() {
        assert_ne!(
            TypeDbTokenStore::key("ab", "c"),
            TypeDbTokenStore::key("a", "bc")
        );
    }
}
