// src/lib.rs
pub mod cache;
pub mod handlers;

mod middleware;
use middleware::RateLimitLayer;
use axum::Router;
use std::sync::Arc;
use tokenizers::Tokenizer;

use handlers::{chat_completions_handler, health_handler};

pub struct AppState {
    pub http_client: reqwest::Client,
    pub backend_urls: Vec<String>,
    pub next_replica: std::sync::atomic::AtomicUsize,
    pub cache: moka::future::Cache<String, String>,
    pub tokenizer: Tokenizer,
    pub latencies: std::sync::RwLock<std::collections::VecDeque<u64>>,
    pub redis_client: Option<redis::Client>,
}

pub async fn app(state: Arc<AppState>) -> axum::Router {
    let rate_limit_layer = RateLimitLayer {
        redis_client: state.redis_client.clone(),
        limit: 100, // 100 req/min
    };

    axum::Router::new()
        .route("/v1/chat/completions", axum::routing::post(chat_completions_handler))
        .layer(rate_limit_layer) //
        .with_state(state)
}

pub fn create_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", axum::routing::get(health_handler))
        // Agora o handler retorna axum::response::Response para suportar zero-copy
        .route("/v1/chat/completions", axum::routing::post(chat_completions_handler))
        .with_state(state)
}