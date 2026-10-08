//! Bound active database-backed HTTP requests and the queue waiting for them.
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::LazyLock;
use tokio::sync::Semaphore;

// The free deployment has a small database and one web instance. This is a
// per-process resource budget, not a replacement for tenant authorization.
pub(crate) fn configured_limit() -> anyhow::Result<usize> {
    let raw = std::env::var("BUSINESS_ACTIVE_REQUESTS").unwrap_or_else(|_| "2".into());
    let limit: usize = raw.parse()?;
    anyhow::ensure!(
        (1..=32).contains(&limit),
        "BUSINESS_ACTIVE_REQUESTS must be between 1 and 32"
    );
    Ok(limit)
}
static ACTIVE: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(configured_limit().expect("validated admission budget")));
static WAITING: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(32));

pub(crate) async fn guard(request: Request, next: Next) -> Response {
    let started = std::time::Instant::now();
    let Ok(waiting) = WAITING.try_acquire() else {
        return overloaded();
    };
    let Ok(Ok(active)) =
        tokio::time::timeout(std::time::Duration::from_secs(5), ACTIVE.acquire()).await
    else {
        return overloaded();
    };
    drop(waiting);
    let queued_ms = started.elapsed().as_secs_f64() * 1000.0;
    let executing = std::time::Instant::now();
    let mut response = next.run(request).await;
    let service_ms = executing.elapsed().as_secs_f64() * 1000.0;
    if let Ok(value) = format!("admission;dur={queued_ms:.3}, service;dur={service_ms:.3}").parse()
    {
        response.headers_mut().insert("server-timing", value);
    }
    drop(active);
    response
}
fn overloaded() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [("retry-after", "1")],
        axum::Json(serde_json::json!({"message":"Service busy; request was not started"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    #[tokio::test]
    async fn full_waiting_queue_rejects_before_handler() {
        let held = WAITING.acquire_many(32).await.unwrap();
        let app = axum::Router::new()
            .route(
                "/test",
                axum::routing::get(|| async {
                    panic!("An overloaded request must not reach the handler");
                    #[allow(unreachable_code)]
                    "unexpected"
                }),
            )
            .route_layer(axum::middleware::from_fn(guard));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["retry-after"], "1");
        drop(held);
    }
}
