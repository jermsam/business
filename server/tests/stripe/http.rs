//! Actual local HTTPS traffic through the production Stripe HTTP implementation.
use super::*;
use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
};
use http_body_util::BodyExt;
use std::{collections::VecDeque, sync::Mutex, time::Duration};

#[derive(Clone)]
struct WireCall {
    method: Method,
    uri: String,
    authorization: String,
    body: Vec<u8>,
    headers: axum::http::HeaderMap,
}
struct Reply {
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
    location: Option<&'static str>,
    delay: Duration,
}
impl Reply {
    fn json(status: StatusCode, body: &str) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.as_bytes().into(),
            location: None,
            delay: Duration::ZERO,
        }
    }
}
struct Fixture {
    base: String,
    client: StripeHttp,
    calls: Arc<Mutex<Vec<WireCall>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new(replies: Vec<Reply>, timeout: Duration) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
        let root = reqwest::Certificate::from_der(cert.cert.der())?;
        let tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der())
                    .into(),
            )?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("https://localhost:{}/v1", listener.local_addr()?.port());
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let capture = calls.clone();
        let router = axum::Router::new().fallback(move |request: Request<Body>| {
            let calls = capture.clone();
            let replies = replies.clone();
            async move {
                let (parts, body) = request.into_parts();
                let bytes = body.collect().await.unwrap().to_bytes();
                calls.lock().unwrap().push(WireCall {
                    method: parts.method,
                    uri: parts.uri.to_string(),
                    authorization: parts
                        .headers
                        .get("authorization")
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    body: bytes.into(),
                    headers: parts.headers,
                });
                let reply = replies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected retry/request");
                tokio::time::sleep(reply.delay).await;
                let mut response = Response::builder()
                    .status(reply.status)
                    .header("content-type", reply.content_type);
                if let Some(location) = reply.location {
                    response = response.header("location", location);
                }
                response.body(Body::from(reply.body)).unwrap()
            }
        });
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let router = router.clone();
                tokio::spawn(async move {
                    if let Ok(tls) = acceptor.accept(stream).await {
                        let _ = hyper_util::server::conn::auto::Builder::new(
                            hyper_util::rt::TokioExecutor::new(),
                        )
                        .serve_connection(
                            hyper_util::rt::TokioIo::new(tls),
                            hyper_util::service::TowerToHyperService::new(router),
                        )
                        .await;
                    }
                });
            }
        });
        let client = StripeHttp {
            http: StripeHttp::client_builder()
                .add_root_certificate(root)
                .timeout(timeout)
                .build()?,
        };
        Ok(Self {
            base,
            client,
            calls,
            task,
        })
    }
    fn access() -> StripeAccess {
        StripeAccess {
            tenant: "tenant-a".into(),
            actor: "alice".into(),
            token: "never-log-this-stripe-token".into(),
            environment: super::super::super::auth_stripe::auth_stripe_schema::Environment::Test,
            write: true,
            connected_account: Some("acct_test".into()),
        }
    }
}

#[test]
fn nested_form_preserves_minor_units_and_clearing() {
    let fields=form(&serde_json::json!({"line_items":[{"price_data":{"unit_amount":123}}],"metadata":{"old":null}})).unwrap();
    assert!(fields.contains(&(
        "line_items[0][price_data][unit_amount]".into(),
        "123".into()
    )));
    assert!(fields.contains(&("metadata[old]".into(), "".into())));
    assert!(form(&serde_json::json!({"invalid[key]":1})).is_err());
}
fn request(method: Method) -> StripeRequest {
    StripeRequest {
        method,
        path: "/customers".into(),
        fields: vec![("metadata[note]".into(), "hello & other=wrong".into())],
        idempotency_key: Some("stable-test-key".into()),
    }
}
#[tokio::test]
async fn real_https_preserves_form_query_headers_and_delete() -> Result<()> {
    let fixture = Fixture::new(
        vec![
            Reply::json(StatusCode::OK, "{}"),
            Reply::json(StatusCode::OK, "{}"),
            Reply::json(StatusCode::OK, "{}"),
        ],
        Duration::from_secs(5),
    )
    .await?;
    let access = Fixture::access();
    for method in [Method::GET, Method::POST, Method::DELETE] {
        fixture
            .client
            .request_at(&fixture.base, &access, request(method))
            .await?;
    }
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    for call in calls.iter() {
        assert_eq!(call.authorization, format!("Bearer {}", access.token));
        assert_eq!(call.headers["stripe-version"], API_VERSION);
        assert_eq!(call.headers["stripe-account"], "acct_test");
    }
    assert_eq!(calls[0].method, Method::GET);
    assert!(calls[0]
        .uri
        .contains("metadata%5Bnote%5D=hello+%26+other%3Dwrong"));
    assert!(!calls[0].headers.contains_key("idempotency-key"));
    assert_eq!(calls[1].headers["idempotency-key"], "stable-test-key");
    assert_eq!(
        calls[1].headers["content-type"],
        "application/x-www-form-urlencoded"
    );
    assert_eq!(
        String::from_utf8(calls[1].body.clone())?,
        "metadata%5Bnote%5D=hello+%26+other%3Dwrong"
    );
    assert!(!calls[2].headers.contains_key("idempotency-key"));
    assert!(calls[2].body.is_empty());
    Ok(())
}
#[tokio::test]
async fn transport_never_follows_redirects_or_retries_uncertain_writes() -> Result<()> {
    for status in [
        StatusCode::FOUND,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::UNAUTHORIZED,
    ] {
        let mut reply = Reply::json(status, "secret-provider-body-do-not-expose");
        reply.location = Some("https://example.invalid/steal");
        let fixture = Fixture::new(vec![reply], Duration::from_secs(5)).await?;
        let error = fixture
            .client
            .request_at(&fixture.base, &Fixture::access(), request(Method::POST))
            .await
            .unwrap_err();
        assert!(!format!("{error:#}").contains("secret-provider-body"));
        assert_eq!(fixture.calls.lock().unwrap().len(), 1);
    }
    let mut reply = Reply::json(StatusCode::OK, "{}");
    reply.delay = Duration::from_millis(250);
    let fixture = Fixture::new(vec![reply], Duration::from_millis(100)).await?;
    assert!(fixture
        .client
        .request_at(&fixture.base, &Fixture::access(), request(Method::POST))
        .await
        .is_err());
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
    Ok(())
}
