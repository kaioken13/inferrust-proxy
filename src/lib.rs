// src/lib.rs
pub mod cache;
pub mod handlers;

pub mod utils;

mod middleware;
use axum::Router;
use middleware::RateLimitLayer;
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

pub fn create_app(state: Arc<AppState>) -> Router {
    // Configure the Rate Limit middleware using the Redis client from the state
    let behind_proxy = std::env::var("BEHIND_PROXY")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let rate_limit_layer = RateLimitLayer {
        redis_client: state.redis_client.clone(),
        limit: 100,
        behind_proxy,
    };

    Router::new()
        .route("/health", axum::routing::get(health_handler))
        .route(
            "/v1/chat/completions",
            axum::routing::post(chat_completions_handler),
        )
        .layer(rate_limit_layer)
        .with_state(state)
}
