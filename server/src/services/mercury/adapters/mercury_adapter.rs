//! Mercury HTTP only. No database, TypeQL, invoice scheduling or local records.
use crate::services::BusinessParams;
use anyhow::Result;
use async_trait::async_trait;
use dog_core::{DogError, TenantContext};
use reqwest::Method;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    Accounts,
    Customers,
    Invoices,
    Transactions,
}
impl StoreKind {
    pub fn path(self) -> &'static str {
        match self {
            Self::Accounts => "/accounts",
            Self::Customers => "/ar/customers",
            Self::Invoices => "/ar/invoices",
            Self::Transactions => "/transactions",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Accounts => "accounts",
            Self::Customers => "customers",
            Self::Invoices => "invoices",
            Self::Transactions => "transactions",
        }
    }
    pub fn mutable(self) -> bool {
        matches!(self, Self::Customers | Self::Invoices)
    }
}
use super::super::auth_mercury::auth_mercury_schema::{Environment, MercuryAccess};
#[async_trait]
pub trait MercuryTransport: Send + Sync {
    async fn request(
        &self,
        access: &MercuryAccess,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Value>,
        pdf: bool,
    ) -> Result<Value>;
}
pub struct MercuryHttp {
    http: reqwest::Client,
}
impl MercuryHttp {
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
}
#[async_trait]
impl MercuryTransport for MercuryHttp {
    async fn request(
        &self,
        access: &MercuryAccess,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Value>,
        pdf: bool,
    ) -> Result<Value> {
        let base = match access.environment {
            Environment::Sandbox => "https://api-sandbox.mercury.com/api/v1",
            Environment::Production => "https://api.mercury.com/api/v1",
        };
        self.request_at(base, access, method, path, query, body, pdf)
            .await
    }
}
impl MercuryHttp {
    // Only fixed Mercury origins reach this helper in production. Tests use a
    // private local TLS fixture to exercise the same wire and response handling.
    #[allow(clippy::too_many_arguments)]
    async fn request_at(
        &self,
        base: &str,
        access: &MercuryAccess,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Value>,
        pdf: bool,
    ) -> Result<Value> {
        let mut request = self
            .http
            .request(method.clone(), format!("{base}{path}"))
            .bearer_auth(&access.token)
            .query(query)
            .header(
                "Accept",
                if pdf {
                    "application/pdf"
                } else {
                    "application/json"
                },
            );
        if let Some(body) = body {
            request = request.json(&body);
        }
        // No automatic retry: a timeout on a mutation can follow provider acceptance.
        let mut response = request.send().await.map_err(|_| {
            DogError::unavailable(
                "Mercury response unavailable; reconcile mutations before retrying",
            )
            .into_anyhow()
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                400 | 422 => DogError::bad_request("Mercury rejected the request"),
                401 | 403 => DogError::forbidden(
                    "Mercury rejected the connection; check token, IP allowlist and permissions",
                ),
                404 => DogError::not_found("Mercury resource not found"),
                409 => {
                    DogError::conflict("Mercury resource cannot be changed in its current state")
                }
                _ => DogError::unavailable(
                    "Mercury request was not confirmed; reconcile mutations before retrying",
                ),
            }
            .into_anyhow());
        }
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        if pdf {
            anyhow::ensure!(
                response
                    .headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .is_some_and(|s| s.starts_with("application/pdf")),
                "Mercury returned a non-PDF response"
            );
        }
        let limit = if pdf {
            10 * 1024 * 1024
        } else {
            2 * 1024 * 1024
        };
        anyhow::ensure!(
            response.content_length().unwrap_or(0) <= limit as u64,
            "Mercury response exceeds download limit"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            DogError::unavailable(
                "Mercury response download failed; reconcile mutations before retrying",
            )
            .into_anyhow()
        })? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= limit,
                "Mercury response exceeds download limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        if pdf {
            anyhow::ensure!(
                bytes.len() <= 10 * 1024 * 1024 && bytes.starts_with(b"%PDF-"),
                "Invalid invoice PDF"
            );
            // DogService<Value> cannot return raw bytes. Preserve them losslessly
            // with an explicit encoding rather than attempting JSON/PDF parsing.
            return Ok(
                json!({"contentType":"application/pdf", "encoding":"base64", "data":base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)}),
            );
        }
        if bytes.is_empty() && method != Method::GET {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            DogError::unavailable(
                "Invalid Mercury JSON response; reconcile mutations before retrying",
            )
            .into_anyhow()
        })
    }
}

#[cfg(test)]
#[path = "../../../../tests/mercury/http.rs"]
mod http_tests;
pub struct MercuryState {
    transport: Arc<dyn MercuryTransport>,
}
impl MercuryState {
    pub fn new(transport: Arc<dyn MercuryTransport>) -> Self {
        Self { transport }
    }
}
pub struct MercuryAdapter {
    transport: Arc<dyn MercuryTransport>,
    pub store: StoreKind,
}
impl MercuryAdapter {
    pub fn new(state: Arc<MercuryState>, store: StoreKind) -> Self {
        Self {
            transport: state.transport.clone(),
            store,
        }
    }
    fn item_path(&self, id: &str) -> Result<String> {
        safe_path_id(id)?;
        Ok(format!(
            "{}/{}",
            if self.store == StoreKind::Transactions {
                "/transaction"
            } else {
                self.store.path()
            },
            id
        ))
    }
    fn bound_resource(value: Value, id: &str) -> Result<Value> {
        if value["id"]
            .as_str()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            != uuid::Uuid::parse_str(id).ok()
        {
            return Err(DogError::unavailable(
                "Mercury returned a different resource; reconcile mutations before retrying",
            )
            .into_anyhow());
        }
        Ok(value)
    }
    pub async fn _find(&self, ctx: &TenantContext, params: BusinessParams) -> Result<Vec<Value>> {
        let query: Vec<_> = params
            .inner
            .query
            .iter()
            .flat_map(|(key, value)| {
                if self.store == StoreKind::Transactions
                    && matches!(key.as_str(), "accountId" | "status" | "cardId")
                {
                    value
                        .split(',')
                        .map(|part| (key.clone(), part.to_string()))
                        .collect::<Vec<_>>()
                } else {
                    vec![(key.clone(), value.clone())]
                }
            })
            .collect();
        let out = self
            .transport
            .request(
                MercuryAccess::for_request(ctx, &params, false)?,
                Method::GET,
                self.store.path(),
                &query,
                None,
                false,
            )
            .await?;
        out.get(self.store.key())
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                DogError::unavailable("Invalid Mercury collection response").into_anyhow()
            })
    }
    pub async fn _get(
        &self,
        ctx: &TenantContext,
        id: &str,
        params: BusinessParams,
    ) -> Result<Value> {
        if self.store == StoreKind::Accounts {
            safe_path_id(id)?;
            let access = MercuryAccess::for_request(ctx, &params, false)?;
            let mut query = vec![("limit".into(), "1000".into())];
            let mut seen = HashSet::new();
            loop {
                let page = self
                    .transport
                    .request(access, Method::GET, "/accounts", &query, None, false)
                    .await?;
                let rows = page["accounts"].as_array().ok_or_else(|| {
                    DogError::unavailable("Invalid Mercury accounts response").into_anyhow()
                })?;
                if let Some(row) = rows.iter().find(|v| v["id"].as_str() == Some(id)) {
                    return Ok(row.clone());
                }
                if page.pointer("/page/nextPage").is_none_or(Value::is_null) {
                    return Err(DogError::not_found("Mercury account not found").into_anyhow());
                }
                let last = page
                    .pointer("/page/nextPage")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("Invalid Mercury account pagination"))?;
                safe_path_id(last)?;
                anyhow::ensure!(
                    seen.insert(last.to_string()) && seen.len() <= 100,
                    "Mercury account pagination did not finish"
                );
                query = vec![
                    ("limit".into(), "1000".into()),
                    ("start_after".into(), last.into()),
                ];
            }
        }
        Self::bound_resource(
            self.transport
                .request(
                    MercuryAccess::for_request(ctx, &params, false)?,
                    Method::GET,
                    &self.item_path(id)?,
                    &[],
                    None,
                    false,
                )
                .await?,
            id,
        )
    }
    pub async fn _create(
        &self,
        ctx: &TenantContext,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        if !self.store.mutable() {
            return Err(DogError::method_not_allowed("Resource is read-only").into_anyhow());
        }
        self.transport
            .request(
                MercuryAccess::for_request(ctx, &params, true)?,
                Method::POST,
                self.store.path(),
                &[],
                Some(data),
                false,
            )
            .await
    }
    pub async fn _update(
        &self,
        ctx: &TenantContext,
        id: &str,
        data: Value,
        params: BusinessParams,
    ) -> Result<Value> {
        if !self.store.mutable() {
            return Err(DogError::method_not_allowed("Resource is read-only").into_anyhow());
        }
        Self::bound_resource(
            self.transport
                .request(
                    MercuryAccess::for_request(ctx, &params, true)?,
                    Method::POST,
                    &self.item_path(id)?,
                    &[],
                    Some(data),
                    false,
                )
                .await?,
            id,
        )
    }
    pub async fn _remove(
        &self,
        ctx: &TenantContext,
        id: Option<&str>,
        params: BusinessParams,
    ) -> Result<Value> {
        if self.store != StoreKind::Customers {
            return Err(
                DogError::method_not_allowed("Only customers support deletion").into_anyhow(),
            );
        }
        let id =
            id.ok_or_else(|| DogError::bad_request("Bulk deletion is forbidden").into_anyhow())?;
        self.transport
            .request(
                MercuryAccess::for_request(ctx, &params, true)?,
                Method::DELETE,
                &self.item_path(id)?,
                &[],
                None,
                false,
            )
            .await?;
        Ok(json!({"id":id,"deleted":true}))
    }
    pub async fn _cancel(
        &self,
        ctx: &TenantContext,
        id: &str,
        params: BusinessParams,
    ) -> Result<Value> {
        anyhow::ensure!(
            self.store == StoreKind::Invoices,
            "Only invoices support cancellation"
        );
        self.transport
            .request(
                MercuryAccess::for_request(ctx, &params, true)?,
                Method::POST,
                &format!("{}/cancel", self.item_path(id)?),
                &[],
                None,
                false,
            )
            .await?;
        Ok(json!({"id":id,"cancelled":true}))
    }
    pub async fn _pdf(
        &self,
        ctx: &TenantContext,
        id: &str,
        params: BusinessParams,
    ) -> Result<Value> {
        anyhow::ensure!(
            self.store == StoreKind::Invoices,
            "Only invoices support PDF downloads"
        );
        self.transport
            .request(
                MercuryAccess::for_request(ctx, &params, false)?,
                Method::GET,
                &format!("{}/pdf", self.item_path(id)?),
                &[],
                None,
                true,
            )
            .await
    }
}

// URL construction safety applies even when an adapter is used directly.
fn safe_path_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| DogError::bad_request("Mercury IDs must be UUIDs").into_anyhow())
}
