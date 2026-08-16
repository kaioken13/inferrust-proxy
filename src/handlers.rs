use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};

use crate::AppState;
use crate::cache::generate_cache_key;
use axum::body::Body;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tracing::{info, error};

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    prompt_tokens: usize,
    response_json: String, // Kept as raw string to avoid parsing overhead
}

pub async fn health_handler() -> impl IntoResponse {
    let health_status = serde_json::json!({
        "status": "ok",
        "message": "inferrust-proxy",
    });
    (StatusCode::OK, Json(health_status))
}

pub async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    
    let cache_key = generate_cache_key(&payload);

    // 1. FAST PATH: Check the Cache First
    if let Some(cached_data) = state.cache.get(&cache_key).await {
        if let Ok(entry) = serde_json::from_str::<CacheEntry>(&cached_data) {
            info!(key = %cache_key, "Cache HIT");
            
            let mut headers = HeaderMap::new();
            headers.insert("x-prompt-tokens", entry.prompt_tokens.into());
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
            
            // Return raw string from cache directly (zero re-serialization)
            return Ok(Response::new(Body::from(entry.response_json)));
        }
    }

    info!(key = %cache_key, "Cache MISS. Processing request...");

    // 2. Extract text and tokenize ONLY on cache miss
    let mut full_text = String::new();
    if let Some(messages) = payload["messages"].as_array() {
        for msg in messages {
            if let Some(content) = msg["content"].as_str() {
                full_text.push_str(content);
                full_text.push(' ');
            }
        }
    }

    let mut prompt_tokens = 0;
    if let Ok(encoding) = state.tokenizer.encode(full_text, true) {
        prompt_tokens = encoding.get_tokens().len();
    }

    // 3. Forward to Backend
    let url = format!("{}/v1/chat/completions", state.backend_url);
    let response = state
        .http_client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            error!(error = %e, "Backend communication failed");
            let error_json = serde_json::json!({
                "error": {
                    "message": "Failed to communicate with the inference backend.",
                    "type": "bad_gateway",
                    "details": e.to_string()
                }
            });
            (reqwest::StatusCode::BAD_GATEWAY, error_json.to_string())
        })?;

    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().await.unwrap_or_default();
        return Err((status, error_body));
    }

    let response_text = response.text().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read backend response: {}", e),
        )
    })?;

    // 4. Save structured payload into Cache (saving token count too!)
    let entry = CacheEntry {
        prompt_tokens,
        response_json: response_text.clone(),
    };
    if let Ok(serialized_entry) = serde_json::to_string(&entry) {
        state.cache.insert(cache_key, serialized_entry).await;
    }

    // 5. Construct Response without deserializing response_text
    let mut headers = HeaderMap::new();
    headers.insert("x-prompt-tokens", prompt_tokens.into());
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));

    Ok(Response::new(Body::from(response_text)))
}