//! Stripe HTTP transport and resource endpoint mapping only.
use super::super::auth_stripe::auth_stripe_schema::StripeAccess;
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogError, TenantContext};
use reqwest::Method;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

// Preserve the version used by the existing integration; no implicit major upgrade.
pub const API_VERSION: &str = "2026-03-25.dahlia";
#[derive(Clone, Debug)]
pub struct StripeRequest {
    pub method: Method,
    pub path: String,
    pub fields: Vec<(String, String)>,
    pub idempotency_key: Option<String>,
}
#[async_trait]
pub trait StripeTransport: Send + Sync {
    async fn request(&self, access: &StripeAccess, request: StripeRequest) -> Result<Value>;
}
pub struct StripeHttp {
    http: reqwest::Client,
}
impl StripeHttp {
    fn client_builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(std::time::Duration::from_secs(25))
    }
    pub fn new() -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Ok(Self {
            http: Self::client_builder().build()?,
        })
    }
    async fn request_at(
        &self,
        base: &str,
        access: &StripeAccess,
        request: StripeRequest,
    ) -> Result<Value> {
        anyhow::ensure!(
            request.path.starts_with('/')
                && !request.path.starts_with("//")
                && !request.path.contains(['?', '#', '%', '\\', '\r', '\n'])
                && !request.path.split('/').any(|p| p == "." || p == ".."),
            "Invalid Stripe path"
        );
        let mut call = self
            .http
            .request(request.method.clone(), format!("{base}{}", request.path))
            .bearer_auth(&access.token)
            .header("Stripe-Version", API_VERSION);
        if let Some(account) = &access.connected_account {
            call = call.header("Stripe-Account", account);
        }
        if request.method == Method::GET {
            call = call.query(&request.fields);
        } else if request.method == Method::POST {
            let key = request
                .idempotency_key
                .as_deref()
                .filter(|k| {
                    !k.is_empty() && k.len() <= 255 && k.bytes().all(|b| (32..=126).contains(&b))
                })
                .ok_or_else(|| {
                    DogError::bad_request("Stripe POST requires an idempotency key").into_anyhow()
                })?;
            call = call.header("Idempotency-Key", key).form(&request.fields);
        } else {
            anyhow::ensure!(
                request.method == Method::DELETE,
                "Unsupported Stripe HTTP method"
            );
        }
        let mut response=call.send().await.map_err(|_|DogError::unavailable("Stripe response unavailable; reconcile using the same operation key before retrying").into_anyhow())?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let mut bytes = Vec::new();
            while let Ok(Some(chunk)) = response.chunk().await {
                if bytes.len() + chunk.len() > 65536 {
                    break;
                }
                bytes.extend_from_slice(&chunk);
            }
            let error: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            // Never expose free-form messages, payment details, bodies or credentials.
            // A parameter is reported only if it is a key we actually submitted.
            let param = error["error"]["param"]
                .as_str()
                .filter(|p| request.fields.iter().any(|(key, _)| key == p));
            let kind = error["error"]["type"].as_str().filter(|s| {
                matches!(
                    *s,
                    "invalid_request_error"
                        | "card_error"
                        | "idempotency_error"
                        | "api_error"
                        | "rate_limit_error"
                )
            });
            let detail = format!(
                "Stripe rejected the operation; payment is not confirmed (type: {}; parameter: {})",
                kind.unwrap_or("unspecified"),
                param.unwrap_or("unspecified")
            );
            return Err(match status {
                400 | 402 | 422 => DogError::bad_request(detail),
                401 | 403 => DogError::forbidden("Stripe connection is not authorized"),
                404 => DogError::not_found("Stripe resource not found"),
                409 => DogError::conflict("Stripe operation conflicts with an existing request"),
                429 => DogError::new(
                    dog_core::errors::ErrorKind::TooManyRequests,
                    "Stripe rate limit reached",
                ),
                _ => DogError::unavailable("Stripe result is uncertain; reconcile before retrying"),
            }
            .into_anyhow());
        }
        let limit = 2 * 1024 * 1024;
        anyhow::ensure!(
            response.content_length().unwrap_or(0) <= limit as u64,
            "Stripe response exceeds limit"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            DogError::unavailable("Stripe response interrupted; reconcile before retrying")
                .into_anyhow()
        })? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= limit,
                "Stripe response exceeds limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        let result: Value = serde_json::from_slice(&bytes).map_err(|_| {
            DogError::unavailable("Invalid Stripe response; reconcile before retrying")
                .into_anyhow()
        })?;
        check_mode(&result, access.live())?;
        Ok(result)
    }
}
#[async_trait]
impl StripeTransport for StripeHttp {
    async fn request(&self, access: &StripeAccess, request: StripeRequest) -> Result<Value> {
        self.request_at("https://api.stripe.com/v1", access, request)
            .await
    }
}
fn check_mode(value: &Value, live: bool) -> Result<()> {
    match value {
        Value::Object(map) => {
            if map.get("object").and_then(Value::as_str).is_some() {
                if let Some(mode) = map.get("livemode") {
                    anyhow::ensure!(
                        mode.as_bool() == Some(live),
                        "Stripe response environment mismatch"
                    );
                }
            }
            for (key, v) in map {
                if key != "metadata" {
                    check_mode(v, live)?;
                }
            }
        }
        Value::Array(rows) => {
            for v in rows {
                check_mode(v, live)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn form(value: &Value) -> Result<Vec<(String, String)>> {
    fn append(
        value: &Value,
        key: String,
        out: &mut Vec<(String, String)>,
        depth: usize,
    ) -> Result<()> {
        anyhow::ensure!(
            depth <= 16 && out.len() < 2048,
            "Stripe form exceeds limits"
        );
        match value {
            Value::Object(map) if !map.is_empty() => {
                for (name, value) in map {
                    anyhow::ensure!(
                        !name.is_empty()
                            && name.len() <= 1000
                            && !name.contains(['[', ']'])
                            && !name.chars().any(char::is_control),
                        "Invalid Stripe parameter name"
                    );
                    append(
                        value,
                        if key.is_empty() {
                            name.clone()
                        } else {
                            format!("{key}[{name}]")
                        },
                        out,
                        depth + 1,
                    )?;
                }
            }
            Value::Array(rows) if !rows.is_empty() => {
                for (index, value) in rows.iter().enumerate() {
                    append(value, format!("{key}[{index}]"), out, depth + 1)?;
                }
            }
            Value::Object(_) | Value::Array(_) | Value::Null => out.push((key, String::new())),
            Value::String(s) => out.push((key, s.clone())),
            _ => out.push((key, value.to_string())),
        }
        Ok(())
    }
    anyhow::ensure!(
        value.is_object() && value.to_string().len() <= 65536,
        "Send one bounded Stripe parameter object"
    );
    if value.as_object().is_some_and(|m| m.is_empty()) {
        return Ok(Vec::new());
    }
    let mut fields = Vec::new();
    append(value, String::new(), &mut fields, 0)?;
    fields.sort();
    Ok(fields)
}
pub struct StripeState {
    transport: Arc<dyn StripeTransport>,
}
impl StripeState {
    pub fn new(transport: Arc<dyn StripeTransport>) -> Self {
        Self { transport }
    }
}
pub struct StripeAdapter {
    transport: Arc<dyn StripeTransport>,
    path: &'static str,
    prefix: &'static str,
}
impl StripeAdapter {
    pub fn new(state: Arc<StripeState>, path: &'static str, prefix: &'static str) -> Self {
        Self {
            transport: state.transport.clone(),
            path,
            prefix,
        }
    }
    fn item(&self, id: &str) -> Result<String> {
        anyhow::ensure!(
            id.starts_with(self.prefix)
                && id.len() > self.prefix.len()
                && id.len() <= 255
                && !id.contains("_secret_")
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "Invalid Stripe resource ID"
        );
        Ok(format!("{}/{id}", self.path))
    }
    async fn send(
        &self,
        ctx: &TenantContext,
        params: BusinessParams,
        method: Method,
        path: String,
        fields: Vec<(String, String)>,
    ) -> Result<Value> {
        let access = StripeAccess::for_request(ctx, &params, method != Method::GET)?;
        let idempotency_key = if method == Method::POST {
            let input = super::super::hooks::request_safety::idempotency_key(&params)?;
            let seed = serde_json::to_vec(&(
                &ctx.tenant_id.0,
                access.live(),
                &access.connected_account,
                &path,
                input,
            ))?;
            Some(format!("business-{:x}", Sha256::digest(seed)))
        } else {
            None
        };
        let result = self
            .transport
            .request(
                access,
                StripeRequest {
                    method,
                    path,
                    fields,
                    idempotency_key,
                },
            )
            .await?;
        check_mode(&result, access.live())?;
        Ok(result)
    }
    pub async fn _find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        let mut fields: Vec<_> = params
            .inner
            .query
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        fields.sort();
        let value = self
            .send(ctx, params, Method::GET, self.path.into(), fields)
            .await?;
        anyhow::ensure!(
            value["object"] == "list" && value["has_more"].is_boolean(),
            "Invalid Stripe collection"
        );
        value["data"]
            .as_array()
            .cloned()
            .ok_or_else(|| DogError::unavailable("Invalid Stripe collection data").into_anyhow())
    }
    pub async fn _get(
        &self,
        ctx: &TenantContext,
        id: &str,
        params: BusinessParams,
    ) -> Result<Value> {
        let result = self
            .send(ctx, params, Method::GET, self.item(id)?, Vec::new())
            .await?;
        bound(result, id)
    }
    pub async fn _create(
        &self,
        ctx: &TenantContext,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        self.send(ctx, params, Method::POST, self.path.into(), form(&data)?)
            .await
    }
    pub async fn _patch(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        let id = id.ok_or_else(|| {
            DogError::bad_request("Bulk Stripe updates are forbidden").into_anyhow()
        })?;
        let result = self
            .send(ctx, params, Method::POST, self.item(id)?, form(&data)?)
            .await?;
        bound(result, id)
    }
    pub async fn _remove(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        params: BusinessParams,
    ) -> Result<Value> {
        let id = id.ok_or_else(|| {
            DogError::bad_request("Bulk Stripe deletion is forbidden").into_anyhow()
        })?;
        bound(
            self.send(ctx, params, Method::DELETE, self.item(id)?, Vec::new())
                .await?,
            id,
        )
    }
    pub async fn _action(
        &self,
        ctx: &TenantContext,
        id: &str,
        action: &'static str,
        data: Value,
        params: BusinessParams,
        read: bool,
    ) -> Result<Value> {
        // Resource services supply the literal action, never an arbitrary client path.
        let path = format!("{}/{}", self.item(id)?, action);
        let result = self
            .send(
                ctx,
                params,
                if read { Method::GET } else { Method::POST },
                path,
                form(&data)?,
            )
            .await?;
        if read {
            Ok(result)
        } else {
            bound(result, id)
        }
    }
}
fn bound(value: Value, id: &str) -> Result<Value> {
    anyhow::ensure!(
        value["id"].as_str() == Some(id),
        "Stripe returned a different resource; reconcile before retrying"
    );
    Ok(value)
}
#[cfg(test)]
#[path = "../../../../tests/stripe/http.rs"]
mod http_tests;
