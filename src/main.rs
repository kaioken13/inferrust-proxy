// src/main.rs
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

    let cache = Cache::builder()
        .max_capacity(10_000)
        .time_to_live(Duration::from_secs(60 * 60))
        .time_to_idle(Duration::from_secs(10 * 60))
        .build();

    let tokenizer = tokenizers::Tokenizer::from_file("tokenizer.json")
        .expect("Failed to load tokenizer.json");

    // Otimização de Pool de conexões TCP/HTTP do Reqwest para evitar handshakes TLS repetidos
    let http_client = Client::builder()
        .pool_max_idle_per_host(50)                    // Mantém conexões quentes com o backend de inferência
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .timeout(Duration::from_secs(300))             // Tempo máximo para LLMs lentas responderem
        .build()
        .expect("Failed to build HTTP Client");

    let state = Arc::new(AppState {
        http_client,
        backend_url: std::env::var("BACKEND_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string()),
        cache,
        tokenizer,
    });

    let app = create_app(state);

    // Permite rodar em 0.0.0.0 em produção (necessário para Docker) e ler porta da Env
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