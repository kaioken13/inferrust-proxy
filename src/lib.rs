use axum::{
    extract::State,
    http::StatusCode,
    Json,
    BoxError,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;
use moka::future::Cache;
use axum::error_handling::HandleErrorLayer;
use std::time::Duration;
use tower::{ServiceBuilder};
use tower::limit::RateLimitLayer;
use sha2::{Sha256, Digest};

// Shared public state so that main.rs and tests can access it
#[derive(Clone)]
pub struct AppState {
    pub http_client: Client,
    pub backend_url: String,
    pub cache: Cache<String, Value>,
}

// Public constructor for the Router (used by main and tests)
pub fn create_app(state: Arc<AppState>) -> axum::Router {
    axum::Router::new()
        .route("/health", axum::routing::get(health_check)) 
        .route("/v1/chat/completions", axum::routing::post(chat_completions_handler))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|err: BoxError| async move {
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        format!("Rate limit exceeded: {}", err),
                    )
                }))
                .layer(tower::buffer::BufferLayer::new(100))
                .layer(RateLimitLayer::new(2, Duration::from_secs(1))),
        )
        .with_state(state)
}

pub async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "inferrust-proxy" }))
}

fn generate_cache_key(payload: &Value) -> String {
    // Extract relevant fields that define a "single request"
    let model = payload["model"].as_str().unwrap_or("default");
    let messages = payload["messages"].to_string();
    let temperature = payload["temperature"].as_f64().unwrap_or(1.0);
    let max_tokens = payload["max_tokens"].as_u64().unwrap_or(2048);
    
    // Create a simplified payload for the hash
    let canonical = format!(
        "{}|{}|{}|{}",
        model, messages, temperature, max_tokens
    );
    //Generate a hash SHA256 (32 bytes) and convert to hexadecimal
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}


async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tracing::info!("Request received at /v1/chat/completions");

    // 1. Generate a cache key
    let cache_key = generate_cache_key(&payload);
    tracing::debug!("Cache key: {}", cache_key);

    // 2. Verify if the request is already in cache
    if let Some(cached_response) = state.cache.get(&cache_key).await {
        tracing::info!("✅ CACHE HIT! Responding in < 2ms.");
        return Ok(Json(cached_response));
    }

    tracing::info!("⏳ CACHE MISS. Sending to backend...");

    // 3. Forward the request to the backend
    let target_url = format!("{}/v1/chat/completions", state.backend_url);
    
    let response = state
        .http_client
        .post(&target_url)
        .json(&payload)
        .send()
        .await
        .map_err(|err| {
            tracing::error!("Error communicating with the backend: {:?}", err);
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": "Failed to communicate with the inference backend.",
                        "type": "bad_gateway",
                        "details": err.to_string()
                    }
                })),
            )
        })?;

    let response_json = response.json::<Value>().await.map_err(|err| {
        tracing::error!("Error deserializing backend response: {:?}", err);
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {
                    "message": "Invalid response received from the inference backend.",
                    "type": "internal_error"
                }
            })),
        )
    })?;

    // 4. Store the response in the cache for future requests
    state.cache.insert(cache_key, response_json.clone()).await;

    Ok(Json(response_json))
}