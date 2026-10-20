//! Tests exercise registered services and real dog-auth JWT verification.
//! Provider fixtures never contact Mercury or create invoices/emails.
use super::adapters::mercury_adapter::*;
use super::auth_mercury::{
    auth_mercury_schema::{Connection, Environment, MercuryAccess},
    AuthMercuryState,
};
use super::*;
use anyhow::Result;
use async_trait::async_trait;
use dog_auth::core::{
    AuthenticationBase, AuthenticationParams, AuthenticationRequest, AuthenticationResult,
    AuthenticationStrategy,
};
use dog_core::{DogApp, DogAppBuilder, HookContext, TenantContext};
use reqwest::Method;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};
const ID: &str = "10000000-0000-4000-8000-000000000001";
const OTHER: &str = "10000000-0000-4000-8000-000000000002";
mod auth_mercury;

#[derive(Clone, Debug)]
struct Call {
    tenant: String,
    credential_digest: String,
    method: Method,
    path: String,
    query: Vec<(String, String)>,
    body: Option<Value>,
    pdf: bool,
}
#[derive(Default)]
struct Provider {
    calls: Mutex<Vec<Call>>,
    responses: Mutex<VecDeque<Value>>,
}
#[async_trait]
impl MercuryTransport for Provider {
    async fn request(
        &self,
        access: &MercuryAccess,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Value>,
        pdf: bool,
    ) -> Result<Value> {
        self.calls.lock().unwrap().push(Call {
            tenant: access.tenant.clone(),
            credential_digest: {
                use sha2::Digest;
                format!("{:x}", sha2::Sha256::digest(access.token.as_bytes()))
            },
            method,
            path: path.into(),
            query: query.into(),
            body,
            pdf,
        });
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| json!({"id":ID})))
    }
}
struct JwtFixture;
#[async_trait]
impl AuthenticationStrategy<BusinessParams> for JwtFixture {
    async fn authenticate(
        &self,
        request: &AuthenticationRequest,
        params: &AuthenticationParams,
        ctx: &mut HookContext<Value, BusinessParams>,
        auth: &AuthenticationBase<BusinessParams>,
    ) -> Result<AuthenticationResult> {
        let mut result = dog_auth::JwtStrategy::new()
            .authenticate(request, params, ctx, auth)
            .await?;
        let payload = &result["payload"];
        // Explicit test membership fixture after real signature/token verification.
        let actor = payload["sub"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing subject"))?;
        let tenant = payload["tenant"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing tenant"))?;
        result["user"] = json!({"id":actor,"tenant_id":tenant});
        Ok(result)
    }
}
struct Harness {
    app: DogApp<Value, BusinessParams>,
    auth: Arc<dog_auth::AuthenticationService<BusinessParams>>,
    provider: Arc<Provider>,
    auth_calls: Arc<std::sync::atomic::AtomicUsize>,
}
impl Harness {
    fn new() -> Result<Self> {
        let provider = Arc::new(Provider::default());
        let connections = vec![
            Connection {
                tenant_id: "tenant-a".into(),
                identity_ids: HashSet::from(["alice".into()]),
                environment: Environment::Sandbox,
                token: "private-a".into(),
                write: true,
            },
            Connection {
                tenant_id: "tenant-b".into(),
                identity_ids: HashSet::from(["bob".into()]),
                environment: Environment::Sandbox,
                token: "private-b".into(),
                write: false,
            },
        ];

        Self::with_transport(connections, provider.clone(), provider)
    }
    fn with_transport(
        connections: Vec<Connection>,
        transport: Arc<dyn MercuryTransport>,
        provider: Arc<Provider>,
    ) -> Result<Self> {
        let mut builder = DogAppBuilder::new();
        let mut options = dog_auth::AuthOptions::default();
        options.jwt.secret = Some("mercury-contract-tests-only-at-least-32-bytes".into());
        let mut base = dog_auth::AuthenticationService::builder(&mut builder, Some(options))?;
        base.register("jwt", Arc::new(JwtFixture));
        let auth = Arc::new(dog_auth::AuthenticationService::new(Arc::new(base.build())));
        let auth_adapter = dog_auth::AuthenticationService::install(&mut builder, auth.clone());
        configure(
            &mut builder,
            auth.clone(),
            Arc::new(MercuryState::new(transport)),
            Arc::new(AuthMercuryState::new(connections)?),
        )?;
        let auth_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        struct CountResolve(Arc<std::sync::atomic::AtomicUsize>);
        #[async_trait]
        impl dog_core::DogBeforeHook<Value, BusinessParams> for CountResolve {
            async fn run(&self, _: &mut HookContext<Value, BusinessParams>) -> Result<()> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
        builder.service_hooks("auth-mercury", |h| {
            h.before_all(Arc::new(CountResolve(auth_calls.clone())));
        });
        let app = builder.build();
        auth_adapter.setup(app.clone());
        Ok(Self {
            app,
            auth,
            provider,
            auth_calls,
        })
    }
    async fn params(&self, actor: &str, tenant: &str) -> Result<BusinessParams> {
        let token = self
            .auth
            .base
            .create_access_token(json!({"sub":actor,"tenant":tenant}), None)
            .await?;
        let mut params = BusinessParams::default();
        params
            .headers
            .insert("authorization".into(), format!("Bearer {token}"));
        Ok(params)
    }
    fn response(&self, value: Value) {
        self.provider.responses.lock().unwrap().push_back(value);
    }
    fn count(&self) -> usize {
        self.provider.calls.lock().unwrap().len()
    }
}
fn customer(update: bool) -> Value {
    let mut value =
        json!({"name":"Test Customer","email":"customer@example.invalid","address":null});
    if update {
        value["resendOpenInvoices"] = json!(false);
    }
    value
}
fn invoice(update: bool) -> Value {
    let mut value = json!({"achDebitEnabled":false,"creditCardEnabled":false,"useRealAccountNumber":false,"ccEmails":[],"invoiceDate":"2026-10-20","dueDate":"2026-11-20","lineItems":[{"name":"Design","quantity":1,"unitPrice":12.34}]});
    if update {
        value["invoiceNumber"] = json!("TEST-1");
        for k in [
            "internalNote",
            "payerMemo",
            "poNumber",
            "servicePeriodStartDate",
            "servicePeriodEndDate",
        ] {
            value[k] = Value::Null;
        }
    } else {
        value["customerId"] = json!(ID);
        value["destinationAccountId"] = json!(OTHER);
        value["sendEmailOption"] = json!("DontSend");
    }
    value
}

#[tokio::test]
async fn crud_and_custom_methods_map_to_provider_resources() -> Result<()> {
    let h = Harness::new()?;
    let ctx = TenantContext::new("tenant-a");
    let p = h.params("alice", "tenant-a").await?;
    for (name, key, path) in [
        ("accounts", "accounts", "/accounts"),
        ("customers", "customers", "/ar/customers"),
        ("invoices", "invoices", "/ar/invoices"),
        ("transactions", "transactions", "/transactions"),
    ] {
        h.response(json!({key:[{"id":ID}],"page":{}}));
        let mut params = p.clone();
        params.inner.query.insert("limit".into(), "25".into());
        assert_eq!(
            h.app
                .service(&format!("mercury/{name}"))?
                .find(ctx.clone(), params)
                .await?
                .len(),
            1
        );
        let call = h.provider.calls.lock().unwrap().last().unwrap().clone();
        assert_eq!(call.method, Method::GET);
        assert_eq!(call.path, path);
        assert!(call.query.contains(&("limit".into(), "25".into())));
        if name == "accounts" {
            h.response(json!({"accounts":[{"id":ID}],"page":{}}));
        }
        assert_eq!(
            h.app
                .service(&format!("mercury/{name}"))?
                .get(ctx.clone(), ID, p.clone())
                .await?["id"],
            ID
        );
    }
    let service = h.app.service("mercury/customers")?;
    service
        .create(ctx.clone(), customer(false), p.clone())
        .await?;
    service
        .update(ctx.clone(), ID, customer(true), p.clone())
        .await?;
    h.response(Value::Null);
    assert_eq!(
        service.remove(ctx.clone(), Some(ID), p.clone()).await?["deleted"],
        true
    );
    let service = h.app.service("mercury/invoices")?;
    let create = invoice(false);
    service
        .create(ctx.clone(), create.clone(), p.clone())
        .await?;
    let update = invoice(true);
    service
        .update(ctx.clone(), ID, update.clone(), p.clone())
        .await?;
    h.response(Value::Null);
    assert_eq!(
        service
            .custom(ctx.clone(), "cancel", Some(json!({"id":ID})), p.clone())
            .await?["cancelled"],
        true
    );
    h.response(json!({"contentType":"application/pdf","encoding":"base64","data":"JVBERi0="}));
    service
        .custom(ctx, "pdf", Some(json!({"id":ID})), p)
        .await?;
    let calls = h.provider.calls.lock().unwrap();
    assert!(calls.iter().any(|c| c.method == Method::POST
        && c.path == "/ar/invoices"
        && c.body.as_ref() == Some(&create)));
    assert!(calls.iter().any(|c| c.method == Method::POST
        && c.path == format!("/ar/invoices/{ID}")
        && c.body.as_ref() == Some(&update)));
    assert!(calls.iter().any(|c| c.path == format!("/transaction/{ID}")));
    assert!(calls
        .iter()
        .any(|c| c.method == Method::GET && c.pdf && c.path.ends_with("/pdf")));
    Ok(())
}

#[tokio::test]
async fn tenant_authentication_permissions_and_forgery_fail_before_provider() -> Result<()> {
    let h = Harness::new()?;
    let service = h.app.service("mercury/customers")?;
    let ctx = TenantContext::new("tenant-a");
    let forged = BusinessParams {
        authenticated: true,
        auth_result: Some(json!({"user":{"id":"alice","tenant_id":"tenant-a"}})),
        ..Default::default()
    };
    assert!(service
        .create(ctx.clone(), customer(false), forged)
        .await
        .is_err());
    for (actor, tenant, requested) in [
        ("bob", "tenant-b", "tenant-a"),
        ("mallory", "tenant-a", "tenant-a"),
        ("alice", "tenant-b", "tenant-b"),
    ] {
        assert!(service
            .get(
                TenantContext::new(requested),
                ID,
                h.params(actor, tenant).await?
            )
            .await
            .is_err());
    }
    assert!(service
        .create(
            TenantContext::new("tenant-b"),
            customer(false),
            h.params("bob", "tenant-b").await?
        )
        .await
        .is_err());
    let mut invalid = h.params("alice", "tenant-a").await?;
    invalid
        .headers
        .insert("authorization".into(), "Bearer invalid".into());
    assert!(service.get(ctx.clone(), ID, invalid).await.is_err());
    assert_eq!(h.count(), 0);
    let mut p = h.params("alice", "tenant-a").await?;
    p.inner.mercury_access = Some(Arc::new(MercuryAccess {
        tenant: "tenant-a".into(),
        actor: "alice".into(),
        token: "do-not-serialize".into(),
        environment: Environment::Sandbox,
        write: false,
    }));
    let serialized = serde_json::to_string(&p)?;
    assert!(!serialized.contains("do-not-serialize") && !serialized.contains("mercury_access"));
    let decoded: BusinessParams = serde_json::from_str(&serialized)?;
    assert!(decoded.inner.mercury_access.is_none());
    Ok(())
}

#[tokio::test]
async fn unsupported_methods_and_invalid_payloads_make_no_provider_calls() -> Result<()> {
    let h = Harness::new()?;
    let ctx = TenantContext::new("tenant-a");
    let p = h.params("alice", "tenant-a").await?;
    for name in ["accounts", "transactions"] {
        let s = h.app.service(&format!("mercury/{name}"))?;
        assert!(s.create(ctx.clone(), json!({}), p.clone()).await.is_err());
        assert!(s
            .update(ctx.clone(), ID, json!({}), p.clone())
            .await
            .is_err());
        assert!(s
            .patch(ctx.clone(), Some(ID), json!({}), p.clone())
            .await
            .is_err());
        assert!(s.remove(ctx.clone(), Some(ID), p.clone()).await.is_err());
    }
    let s = h.app.service("mercury/customers")?;
    assert!(s
        .patch(ctx.clone(), Some(ID), customer(false), p.clone())
        .await
        .is_err());
    assert!(s.remove(ctx.clone(), None, p.clone()).await.is_err());
    assert!(s.get(ctx.clone(), "../accounts", p.clone()).await.is_err());
    let mut bad = customer(false);
    bad["token"] = json!("override");
    assert!(s.create(ctx.clone(), bad, p.clone()).await.is_err());
    let s = h.app.service("mercury/invoices")?;
    assert!(s
        .patch(
            ctx.clone(),
            Some(ID),
            json!({"payerMemo":"partial"}),
            p.clone()
        )
        .await
        .is_err());
    assert!(s.remove(ctx.clone(), Some(ID), p.clone()).await.is_err());
    assert!(s
        .update(
            ctx.clone(),
            ID,
            json!({"invoiceNumber":"incomplete"}),
            p.clone()
        )
        .await
        .is_err());
    assert!(s
        .custom(
            ctx.clone(),
            "cancel",
            Some(json!({"id":ID,"url":"https://attacker.invalid"})),
            p.clone()
        )
        .await
        .is_err());
    let mut bad = invoice(false);
    bad["lineItems"][0]["unitPrice"] = json!(12.345);
    assert!(s.create(ctx.clone(), bad, p.clone()).await.is_err());
    let mut bad = invoice(false);
    bad.as_object_mut().unwrap().remove("sendEmailOption");
    assert!(s.create(ctx.clone(), bad, p.clone()).await.is_err());
    for query in [
        HashMap::from([("limit".into(), "1001".into())]),
        HashMap::from([("url".into(), "https://attacker.invalid".into())]),
        HashMap::from([
            ("start_after".into(), ID.into()),
            ("end_before".into(), OTHER.into()),
        ]),
    ] {
        let mut params = p.clone();
        params.inner.query = query;
        assert!(s.find(ctx.clone(), params).await.is_err());
    }
    assert_eq!(h.count(), 0);
    Ok(())
}

#[tokio::test]
async fn account_lookup_follows_provider_cursor_and_stops_on_repeated_cursor() -> Result<()> {
    let h = Harness::new()?;
    let p = h.params("alice", "tenant-a").await?;
    let s = h.app.service("mercury/accounts")?;
    h.response(json!({"accounts":[{"id":OTHER}],"page":{"nextPage":OTHER}}));
    h.response(json!({"accounts":[{"id":ID}],"page":{}}));
    assert_eq!(
        s.get(TenantContext::new("tenant-a"), ID, p.clone()).await?["id"],
        ID
    );
    assert!(h.provider.calls.lock().unwrap()[1]
        .query
        .contains(&("start_after".into(), OTHER.into())));
    for _ in 0..2 {
        h.response(json!({"accounts":[{"id":OTHER}],"page":{"nextPage":OTHER}}));
    }
    assert!(s.get(TenantContext::new("tenant-a"), ID, p).await.is_err());
    assert_eq!(h.count(), 4);
    Ok(())
}

#[tokio::test]
async fn http_routes_use_full_resource_names_and_enforce_hooks() -> Result<()> {
    use dog_transport::IntoDogService;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let h = Harness::new()?;
    let http = h.app.clone().into_service(
        dog_transport::HttpOptions::default()
            .tenant_header("x-tenant-id")
            .body_limit(65536),
    );
    let router = super::router(http);
    let p = h.params("alice", "tenant-a").await?;
    let authorization = &p.headers["authorization"];
    for (path, key) in [
        ("accounts", "accounts"),
        ("customers", "customers"),
        ("invoices", "invoices"),
        ("transactions", "transactions"),
    ] {
        h.response(json!({key:[{"id":ID}],"page":{}}));
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/mercury/{path}?limit=25"))
                    .header("authorization", authorization)
                    .header("x-tenant-id", "tenant-a")
                    .body(axum::body::Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), 200);
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await?.to_bytes())?;
        assert!(body.to_string().contains(ID));
    }
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/mercury/transactions/{ID}"))
                .header("authorization", authorization)
                .header("x-tenant-id", "tenant-a")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), 200);
    assert_eq!(
        h.provider.calls.lock().unwrap().last().unwrap().path,
        format!("/transaction/{ID}")
    );
    let count = h.count();
    for (method, path, auth, tenant) in [
        ("GET", "/mercury/accounts", false, "tenant-a"),
        ("GET", "/mercury/accounts", true, "tenant-b"),
        ("POST", "/mercury/accounts", true, "tenant-a"),
        ("DELETE", "/mercury/customers", true, "tenant-a"),
    ] {
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("x-tenant-id", tenant);
        if auth {
            request = request.header("authorization", authorization);
        }
        let response = router
            .clone()
            .oneshot(request.body(axum::body::Body::empty())?)
            .await?;
        assert!(!response.status().is_success());
    }
    assert_eq!(h.count(), count);
    h.response(Value::Null);
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/mercury/invoices")
                .header("authorization", authorization)
                .header("x-tenant-id", "tenant-a")
                .header("x-service-method", "cancel")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(json!({"id":ID}).to_string()))?,
        )
        .await?;
    assert_eq!(response.status(), 200);
    assert!(h
        .provider
        .calls
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .path
        .ends_with("/cancel"));
    let count = h.count();
    let response = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/mercury/invoices")
                .header("authorization", authorization)
                .header("x-tenant-id", "tenant-a")
                .header("x-service-method", "cancel")
                .body(axum::body::Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), 405);
    assert_eq!(h.count(), count);
    Ok(())
}

#[tokio::test]
async fn provider_results_are_checked_and_read_only_connections_can_read() -> Result<()> {
    let h = Harness::new()?;
    let s = h.app.service("mercury/customers")?;
    let p = h.params("bob", "tenant-b").await?;
    assert_eq!(
        s.get(TenantContext::new("tenant-b"), ID, p).await?["id"],
        ID
    );
    let p = h.params("alice", "tenant-a").await?;
    h.response(json!({"id":OTHER}));
    assert!(s
        .get(TenantContext::new("tenant-a"), ID, p.clone())
        .await
        .is_err());
    h.response(json!({"id":OTHER}));
    assert!(s
        .update(
            TenantContext::new("tenant-a"),
            ID,
            customer(true),
            p.clone()
        )
        .await
        .is_err());
    h.response(json!({"customers":[{"id":"invalid-provider-id"}]}));
    assert!(s
        .find(TenantContext::new("tenant-a"), p.clone())
        .await
        .is_err());
    h.response(json!({"unexpected":"shape"}));
    assert!(s.find(TenantContext::new("tenant-a"), p).await.is_err());
    assert_eq!(h.count(), 5);
    Ok(())
}
