// src/lib.rs
pub mod cache;
pub mod handlers;

use axum::Router;
use moka::future::Cache;
use reqwest::Client;
use std::sync::Arc;
use tokenizers::Tokenizer;

use handlers::{chat_completions_handler, health_handler};

pub struct AppState {
    pub http_client: Client,
    pub backend_url: String,
    pub cache: Cache<String, String>,
    pub tokenizer: Tokenizer,
}

pub fn create_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", axum::routing::get(health_handler))
        // Agora o handler retorna axum::response::Response para suportar zero-copy
        .route("/v1/chat/completions", axum::routing::post(chat_completions_handler))
        .with_state(state)
}