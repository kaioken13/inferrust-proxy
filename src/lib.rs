// src/lib.rs

// 1. Declare your new modules so the compiler knows they exist
pub mod cache;
pub mod handlers;

use axum::{error_handling::HandleErrorLayer, BoxError, Router};
use moka::future::Cache;
use reqwest::Client;
use std::{sync::Arc, time::Duration};
use tower::{limit::RateLimitLayer, ServiceBuilder};
use tokenizers::Tokenizer;

// 2. Import your handlers from the new module
use handlers::{chat_completions_handler, health_handler};

// 3. Define the Global State
pub struct AppState {
    pub http_client: Client,
    pub backend_url: String,
    pub cache: Cache<String, String>,
    pub tokenizer: Tokenizer,
}

// 4. Configure the Router and Middleware
pub fn create_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", axum::routing::get(health_handler))
        .route("/v1/chat/completions", axum::routing::post(chat_completions_handler))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|err: BoxError| async move {
                    (
                        reqwest::StatusCode::TOO_MANY_REQUESTS,
                        format!("Rate limit exceeded: {}", err),
                    )
                }))
                .layer(tower::buffer::BufferLayer::new(100))
                .layer(RateLimitLayer::new(2, Duration::from_secs(1))),
        )
        .with_state(state)
}