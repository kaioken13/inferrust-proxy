// tests/common/mod.rs
#![allow(dead_code)]

use inferrust_proxy::{create_app, AppState};
use moka::future::Cache;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

/// Creates a test instance with a specific Cache TTL (Time to Live)
pub fn setup_test_app_with_ttl(backend_url: String, ttl_secs: u64) -> axum::Router {
    let tokenizer = tokenizers::Tokenizer::from_file("tokenizer.json")
        .expect("Failed to load tokenizer.json");

    // Configure the Moka cache with a specific TTL
    let cache = Cache::builder()
        .max_capacity(100)
        .time_to_live(Duration::from_secs(ttl_secs))
        .build();

    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_urls: vec![backend_url],
        cache,
        tokenizer,
        next_replica: std::sync::atomic::AtomicUsize::new(0),
        latencies: std::sync::RwLock::new(std::collections::VecDeque::with_capacity(100)),
        redis_client: None, // Redis mocked out for these tests
    });
    
    create_app(state)
}

/// Creates a test instance of the Axum router with a default long TTL (1 hour) for standard tests.
pub fn setup_test_app(backend_url: String) -> axum::Router {
    setup_test_app_with_ttl(backend_url, 3600)
}

/// Creates a test instance with a REAL Redis client for Rate Limiter tests
pub fn setup_test_app_with_redis(backend_url: String) -> axum::Router {
    let tokenizer = tokenizers::Tokenizer::from_file("tokenizer.json")
        .expect("Failed to load tokenizer.json");

    let cache = Cache::builder().max_capacity(100).build();

    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_urls: vec![backend_url],
        cache,
        tokenizer,
        next_replica: std::sync::atomic::AtomicUsize::new(0),
        latencies: std::sync::RwLock::new(std::collections::VecDeque::with_capacity(100)),
        // ATENÇÃO: Aqui nós ativamos o Redis real para o teste!
        redis_client: Some(redis::Client::open("redis://127.0.0.1:6379").unwrap()),
    });
    
    create_app(state)
}