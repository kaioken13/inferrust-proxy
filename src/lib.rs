// src/lib.rs
pub mod cache;
pub mod handlers;

pub mod utils;
pub mod vectordb_client;

mod middleware;

use crate::handlers::search_vectors;
use crate::vectordb_client::VectorServiceClient;
use axum::{routing::post, Router};
use middleware::RateLimitLayer;
use std::sync::Arc;
use tokenizers::Tokenizer;
use tonic::transport::Channel;

use handlers::{chat_completions_handler, health_handler};

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
    // Configure the Rate Limit middleware using the Redis client from the state
    let behind_proxy = std::env::var("BEHIND_PROXY")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    // Load the rate limit from the environment variable, defaulting to 100 if not set or invalid
    let rate_limit = std::env::var("RATE_LIMIT_BURST")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| {
            tracing::info!("RATE_LIMIT_BURST not defined or invalid. Using default: 100");
            100
        });

    let rate_limit_layer = RateLimitLayer {
        redis_client: state.redis_client.clone(),
        limit: rate_limit,
        behind_proxy,
    };

    Router::new()
        .route("/health", axum::routing::get(health_handler))
        .route(
            "/v1/chat/completions",
            axum::routing::post(chat_completions_handler),
        )
        .route("/v1/vectors/search", post(search_vectors))
        .layer(rate_limit_layer)
        .with_state(state)
}
