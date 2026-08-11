use inferrust_proxy::{create_app, AppState};
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "inferrust_proxy=debug,info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_url: std::env::var("BACKEND_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string()),
    });

    let app = create_app(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Proxy rodando em http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}