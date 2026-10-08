use anyhow::Result;
#[tokio::main]
async fn main() -> Result<()> {
    let (app, http) = server::build().await?;
    let host: String = app.get("http.host").unwrap();
    let port: String = app.get("http.port").unwrap();
    let router = server::app::router(http).merge(server::app::billing_router(&app)?);
    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}")).await?;
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
    Ok(())
}
