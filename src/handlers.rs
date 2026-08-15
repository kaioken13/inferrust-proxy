// src/handlers.rs
use crate::AppState;
use crate::cache::generate_cache_key;
use axum::{
    extract::State,
    Json,
    http::HeaderMap,
    response::IntoResponse,
};
use reqwest::StatusCode;
use serde_json::Value;
use std::sync::Arc;

pub async fn health_handler() -> Json<Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "inferrust-proxy"
    }))
}

// In src/handlers.rs

pub async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    
    // 1. Generate the unique Cache Key
    let cache_key = generate_cache_key(&payload);

    // 2. Extract text and count tokens
    let mut full_text = String::new();
    if let Some(messages) = payload["messages"].as_array() {
        for msg in messages {
            if let Some(content) = msg["content"].as_str() {
                full_text.push_str(content);
                full_text.push(' ');
            }
        }
    }

    let mut headers = HeaderMap::new();
    if let Ok(encoding) = state.tokenizer.encode(full_text, true) {
        let token_count = encoding.get_tokens().len();
        println!("💰 Prompt Token Count: {}", token_count);
        headers.insert(
            "x-prompt-tokens",
            token_count.to_string().parse().unwrap(),
        );
    }

    // 3. Check the Cache (HIT)
    if let Some(cached_response) = state.cache.get(&cache_key).await {
        if let Ok(parsed_json) = serde_json::from_str(&cached_response) {
            println!("🟢 Cache HIT for key: {}", cache_key);
            // Return headers + JSON
            return Ok((headers, Json(parsed_json)));
        }
    }

    // 4. Cache (MISS) - Forward to Backend
    println!("🔴 Cache MISS. Forwarding to backend...");
    let url = format!("{}/v1/chat/completions", state.backend_url);
    
    let response = state
        .http_client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
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

    // 5. Save the response into the Cache
    state.cache.insert(cache_key, response_text.clone()).await;

    // 6. Parse and return the JSON
    let json_response: Value = serde_json::from_str(&response_text).map_err(|e| {
        (
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to parse response as JSON: {}", e),
        )
    })?;

    // Return headers + JSON
    Ok((headers, Json(json_response)))
}