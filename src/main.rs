use inferrust_proxy::{create_app, AppState};
use moka::future::Cache;
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "inferrust_proxy=debug,info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 1. Configuring the Cache policies
    let cache = Cache::builder()
    .max_capacity(10_000) // Maximum number of entries in the cache
    .time_to_live(Duration::from_secs(60 * 60)) // Entries will live for 60 minutes
    .time_to_idle(Duration::from_secs(10 * 60)) // Entries will be removed if not accessed for 10 minutes
    .build();

    let tokenizer = tokenizers::Tokenizer::from_file("tokenizer.json")
        .expect("Failed to load tokenizer.json");

    // 2. Creating the shared application state
    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_url: std::env::var("BACKEND_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string()),
        cache,
        tokenizer,
    });

    let app = create_app(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Proxy running at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}