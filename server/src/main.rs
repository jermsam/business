use anyhow::Result;
use server::build;

#[tokio::main]
async fn main() -> Result<()> {
    let axum_app = build().await?;
    let host: String = axum_app.app.get("http.host").unwrap();
    let port: String = axum_app.app.get("http.port").unwrap();
    let addr = format!("{host}:{port}");

    println!("[wassaid] listening on http://{addr}");
    axum_app.listen(&addr).await?;
    Ok(())
}
