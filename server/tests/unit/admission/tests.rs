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
