use anyhow::Result;
#[tokio::main]
async fn main() -> Result<()> {
    let (app, http) = server::build().await?;
    let host: String = app.get("http.host").unwrap();
    let port: String = app.get("http.port").unwrap();
    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}")).await?;
    let mut router = server::app::router(http);
    let mut billing_worker = None;
    if std::env::var("BILLING_ENABLED").as_deref() == Ok("true") {
        let state = app
            .get("typedb")
            .ok_or_else(|| anyhow::anyhow!("Missing TypeDB state"))?;
        let engine = server::billing::engine::Engine::configured(state)?;
        let (billing_routes, worker) = server::billing::http::runtime(engine)?;
        billing_worker = Some(worker);
        router = router.merge(billing_routes);
    }
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("Install SIGTERM handler");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {},
                    _ = terminate.recv() => {},
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        })
        .await?;
    if let Some(worker) = billing_worker {
        worker.abort();
        let _ = worker.await;
    }
    Ok(())
}
