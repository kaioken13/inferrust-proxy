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
        backend_urls: vec![backend_url],
        cache: Cache::new(100),
        tokenizer,
        next_replica: std::sync::atomic::AtomicUsize::new(0),
        latencies: std::sync::RwLock::new(std::collections::VecDeque::with_capacity(100)),
        redis_client: redis::Client::open("redis://127.0.0.1:6379").unwrap(),
    });
    
    create_app(state)
}