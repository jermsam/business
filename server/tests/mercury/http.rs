//! Actual local HTTPS traffic through the production Mercury HTTP implementation.
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
    client: MercuryHttp,
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
        let base = format!("https://localhost:{}/api/v1", listener.local_addr()?.port());
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
        let client = MercuryHttp {
            http: MercuryHttp::client_builder()
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
    fn access() -> MercuryAccess {
        MercuryAccess {
            tenant: "tenant-a".into(),
            actor: "alice".into(),
            token: "never-log-this-mercury-token".into(),
            environment: Environment::Sandbox,
            write: true,
        }
    }
}

#[tokio::test]
async fn real_https_preserves_auth_query_json_and_pdf_bytes() -> Result<()> {
    let pdf = b"%PDF-1.7\n\x00\xff exact binary\n%%EOF";
    let fixture = Fixture::new(
        vec![
            Reply::json(StatusCode::OK, "{\"transactions\":[],\"page\":{}}"),
            Reply::json(StatusCode::OK, "{\"id\":\"created\"}"),
            Reply {
                status: StatusCode::OK,
                content_type: "application/pdf",
                body: pdf.into(),
                location: None,
                delay: Duration::ZERO,
            },
            Reply::json(StatusCode::OK, ""),
        ],
        Duration::from_secs(5),
    )
    .await?;
    let access = Fixture::access();
    let query = vec![("search".into(), "hello & category=wrong".into())];
    fixture
        .client
        .request_at(
            &fixture.base,
            &access,
            Method::GET,
            "/transactions",
            &query,
            None,
            false,
        )
        .await?;
    let body = json!({"name":"Test Customer","email":"test@example.invalid"});
    fixture
        .client
        .request_at(
            &fixture.base,
            &access,
            Method::POST,
            "/ar/customers",
            &[],
            Some(body.clone()),
            false,
        )
        .await?;
    let result = fixture
        .client
        .request_at(
            &fixture.base,
            &access,
            Method::GET,
            "/ar/invoices/test/pdf",
            &[],
            None,
            true,
        )
        .await?;
    assert_eq!(
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            result["data"].as_str().unwrap()
        )?,
        pdf
    );
    assert!(fixture
        .client
        .request_at(
            &fixture.base,
            &access,
            Method::DELETE,
            "/ar/customers/test",
            &[],
            None,
            false
        )
        .await?
        .is_null());
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert!(calls
        .iter()
        .all(|c| c.authorization == format!("Bearer {}", access.token)));
    assert_eq!(calls[1].method, Method::POST);
    assert_eq!(serde_json::from_slice::<Value>(&calls[1].body)?, body);
    let url = reqwest::Url::parse(&format!("https://localhost{}", calls[0].uri))?;
    assert_eq!(
        url.query_pairs().collect::<Vec<_>>(),
        vec![("search".into(), "hello & category=wrong".into())]
    );
    Ok(())
}

#[tokio::test]
async fn real_https_errors_are_redacted_and_redirects_are_not_followed() -> Result<()> {
    let mut redirect = Reply::json(StatusCode::FOUND, "sensitive-provider-body");
    redirect.location = Some("https://example.invalid/steal");
    let fixture = Fixture::new(
        vec![
            Reply::json(StatusCode::UNAUTHORIZED, "never-log-this-mercury-token"),
            Reply::json(StatusCode::TOO_MANY_REQUESTS, "sensitive-provider-body"),
            redirect,
            Reply::json(StatusCode::OK, "private invalid json"),
        ],
        Duration::from_secs(5),
    )
    .await?;
    for _ in 0..4 {
        let error = fixture
            .client
            .request_at(
                &fixture.base,
                &Fixture::access(),
                Method::POST,
                "/ar/invoices",
                &[],
                Some(json!({})),
                false,
            )
            .await
            .unwrap_err();
        let text = format!("{error:#}");
        assert!(
            !text.contains("never-log-this")
                && !text.contains("sensitive-provider")
                && !text.contains("private invalid")
        );
    }
    assert_eq!(fixture.calls.lock().unwrap().len(), 4);
    Ok(())
}

#[tokio::test]
async fn timed_out_mutation_is_not_retried() -> Result<()> {
    let mut reply = Reply::json(StatusCode::OK, "{}");
    reply.delay = Duration::from_secs(10);
    let fixture = Fixture::new(vec![reply], Duration::from_secs(2)).await?;
    let error = fixture
        .client
        .request_at(
            &fixture.base,
            &Fixture::access(),
            Method::POST,
            "/ar/invoices",
            &[],
            Some(json!({})),
            false,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("reconcile"));
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
    Ok(())
}
