#[tokio::main]
async fn main() -> anyhow::Result<()> {
    server::operations::run().await
}
