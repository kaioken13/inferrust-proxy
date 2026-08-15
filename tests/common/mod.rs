// tests/common/mod.rs
use inferrust_proxy::{create_app, AppState};
use moka::future::Cache;
use reqwest::Client;
use std::sync::Arc;

/// Creates a test instance of the Axum router with a configurable backend URL.
pub fn setup_test_app(backend_url: String) -> axum::Router {
    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_url,
        cache: Cache::new(100),
        // If you already added the Tokenizer in Phase 4, you will need to initialize it here too!
    });
    
    create_app(state)
}