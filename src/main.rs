// src/main.rs
use inferrust_proxy::{create_app, AppState};
use moka::future::Cache;
use reqwest::Client;
use std::net::SocketAddr;
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

    let cache = Cache::builder()
        .max_capacity(10_000)
        .time_to_live(Duration::from_secs(60 * 60))
        .time_to_idle(Duration::from_secs(10 * 60))
        .build();

    let tokenizer = tokenizers::Tokenizer::from_file("tokenizer.json")
        .expect("Failed to load tokenizer.json");

    // Reqwest TCP/HTTP Connection Pool Optimization to avoid repeated TLS handshakes
    let http_client = Client::builder()
        .pool_max_idle_per_host(50)                    // Maintains warm connections to the inference backend
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        // TODO: Mitigate Latency Kills
        .timeout(Duration::from_secs(300))             // Maximum time for slow LLMs to respond
        .build()
        .expect("Failed to build HTTP Client");

    let backend_urls_env = std::env::var("BACKEND_URLS")
    .unwrap_or_else(|_| "http://localhost:11434".to_string());

    let backend_urls: Vec<String> = backend_urls_env
        .split(',')
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .collect();

    let redis_client = redis::Client::open("redis://127.0.0.1/")
        .expect("Failed to initialize Redis client");

    let state = std::sync::Arc::new(crate::AppState {
        http_client,
        backend_urls,
        next_replica: std::sync::atomic::AtomicUsize::new(0),
        cache,
        tokenizer,
        latencies: std::sync::RwLock::new(std::collections::VecDeque::with_capacity(100)),
        redis_client: Some(redis_client),
    });

    let app = create_app(state);

    // Allows running on 0.0.0.0 in production (required for Docker) and reading the Env port
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .expect("Invalid address configuration");

    tracing::info!("Proxy running at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}