use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;
use moka::future::Cache;

// Shared public state so that main.rs and tests can access it
#[derive(Clone)]
pub struct AppState {
    pub http_client: Client,
    pub backend_url: String,
    pub cache: Cache<String, Value>,
}

// Public constructor for the Router (used by main and tests)
pub fn create_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .with_state(state)
}

pub async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "inferrust-proxy" }))
}

pub async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tracing::info!("Request received at /v1/chat/completions");

    let target_url = format!("{}/v1/chat/completions", state.backend_url);

    // 1. Generate a cache key
    let cache_key = match serde_json::to_string(&payload["messages"]) {
        Ok(key) => key,
        Err(_) => "invalid_key".to_string(),
    };

    // 2. Verify cache (Cache HIT)
    if let Some(cached_response) = state.cache.get(&cache_key).await {
        tracing::info!("🟢 CACHE HIT! Returning response from RAM instantly.");
        return Ok(Json(cached_response));
    }

    tracing::info!("🔴 CACHE MISS. Forwarding request to the inference backend...");

    // Send to Ollama
    let response = state
        .http_client
        .post(&target_url)
        .json(&payload)
        .send()
        .await
        .map_err(|err| {
            tracing::error!("Error communicating with the inference backend: {:?}", err);
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": "Communication failure with the inference backend..",
                        "type": "bad_gateway",
                        "details": err.to_string()
                    }
                })),
            )
        })?;

    let response_json = response.json::<Value>().await.map_err(|err| {
        tracing::error!("Error deserialize backend response: {:?}", err);
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

    // 3. save to cache (Cache INSERT)
    state.cache.insert(cache_key, response_json.clone()).await;

    Ok(Json(response_json))
}