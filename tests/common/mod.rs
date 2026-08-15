// tests/common/mod.rs
use inferrust_proxy::{create_app, AppState};
use moka::future::Cache;
use reqwest::Client;
use std::sync::Arc;

/// Creates a test instance of the Axum router with a configurable backend URL.
pub fn setup_test_app(backend_url: String) -> axum::Router {
    let tokenizer = tokenizers::Tokenizer::from_file("tokenizer.json")
        .expect("Failed to load tokenizer.json");

    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_url,
        cache: Cache::new(100),
        tokenizer,
    });
    
    create_app(state)
}