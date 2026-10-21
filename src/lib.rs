// src/lib.rs
pub mod cache;
pub mod handlers;
pub mod middleware;
pub mod rate_limiter;
pub mod utils;
pub mod vectordb_client;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tokenizers::Tokenizer;
use tonic::transport::Channel;

use crate::handlers::{chat_completions_handler, health_handler, search_vectors};
use crate::middleware::RateLimitLayer;
use crate::rate_limiter::redis_backend::RedisRateLimiter;
use crate::rate_limiter::RateLimiter;
use crate::vectordb_client::VectorServiceClient;

pub struct AppState {
    pub http_client: reqwest::Client,
    pub backend_urls: Vec<String>,
    pub next_replica: std::sync::atomic::AtomicUsize,
    pub cache: moka::future::Cache<String, String>,
    pub tokenizer: Tokenizer,
    pub latencies: std::sync::RwLock<std::collections::VecDeque<u64>>,
    pub redis_client: Option<redis::Client>,
    pub vectordb_client: VectorServiceClient<Channel>,
}

pub fn create_app(state: Arc<AppState>) -> Router {
    let behind_proxy = std::env::var("BEHIND_PROXY")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let rate_limit = std::env::var("RATE_LIMIT_BURST")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| {
            tracing::info!("RATE_LIMIT_BURST not defined or invalid. Using default: 100");
            100
        });

    let window_secs = std::env::var("RATE_LIMIT_WINDOW_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60);

    // Constrói o limiter via trait a partir do RedisClient registrado no AppState
    let limiter: Option<Arc<dyn RateLimiter>> = state
        .redis_client
        .as_ref()
        .map(|client| Arc::new(RedisRateLimiter::new(client.clone())) as Arc<dyn RateLimiter>);

    let rate_limit_layer = RateLimitLayer {
        limiter,
        limit: rate_limit,
        window_secs,
        behind_proxy,
    };

    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .route("/v1/vectors/search", post(search_vectors))
        .layer(rate_limit_layer)
        .with_state(state)
}
