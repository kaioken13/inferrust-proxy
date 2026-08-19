use axum::Json;
use axum::response::IntoResponse;
use crate::AppState;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use sha2::Digest;

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    prompt_tokens: usize,
    response_json: String, // Kept as raw string to avoid parsing overhead
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub async fn health_handler() -> impl IntoResponse {
    let health_status = serde_json::json!({
        "status": "ok",
        "message": "inferrust-proxy",
    });
    (StatusCode::OK, Json(health_status))
}

fn get_p95_timeout(state: &AppState) -> u64 {
    let latencies = state.latencies.read().unwrap();
    if latencies.len() < 10 { return 2500; }

    let mut sorted = latencies.iter().cloned().collect::<Vec<_>>(); // Com <Vec<_>>
    sorted.sort_unstable();
    
    let index = (sorted.len() as f64 * 0.95) as usize;
    let p95 = sorted[index];

    p95.clamp(500, 5000)
}

pub async fn chat_completions_handler(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<AppState>>,
    axum::Json(payload): axum::Json<serde_json::Value>,
) -> Result<impl axum::response::IntoResponse, (axum::http::StatusCode, String)> {
    
    // 1. Capture start time for dynamic p95 calculation
    let start = std::time::Instant::now();

    // 2. Generate Cache Key using Bincode/SHA-256
    let mut hasher = sha2::Sha256::new();
    if let Ok(canonical_messages) = serde_json::from_value::<Vec<ChatMessage>>(payload["messages"].clone()) {
        if let Ok(binary_bytes) = bincode::serialize(&canonical_messages) {
            sha2::Digest::update(&mut hasher, &binary_bytes);
        } else {
            tracing::warn!("Failed to serialize messages to binary.");
        }
    } else {
        tracing::warn!("Payload does not contain a valid messages format for caching.");
    }
    let cache_key = format!("{:x}", hasher.finalize());

    // 3. FAST PATH: Cache HIT
    if let Some(cached_data) = state.cache.get(&cache_key).await {
        if let Ok(entry) = serde_json::from_str::<CacheEntry>(&cached_data) {
            tracing::info!(key = %cache_key, "Cache HIT");
            
            let mut headers = axum::http::HeaderMap::new();
            headers.insert("x-prompt-tokens", entry.prompt_tokens.into());
            headers.insert(axum::http::header::CONTENT_TYPE, axum::http::HeaderValue::from_static("application/json"));
            
            return Ok((headers, axum::body::Body::from(entry.response_json)).into_response());
        }
    }

    tracing::info!(key = %cache_key, "Cache MISS. Processing request...");

    // 4. Prepare Idempotency, Dynamic p95 Timeout, and Load Balancing
    let idempotency_key = uuid::Uuid::new_v4().to_string();
    tracing::debug!("Generating Idempotency-Key: {}", idempotency_key);

    let total_replicas = state.backend_urls.len();
    let current_idx = state.next_replica.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let idx_a = current_idx % total_replicas;
    let url_a = format!("{}/v1/chat/completions", state.backend_urls[idx_a]);
    tracing::debug!("Routing Request A to replica [{}]", url_a);

    let client = state.http_client.clone();
    let payload_clone = payload.clone();

    // 5. Build Request A (Original)
    let req_a = async { 
        client.post(&url_a)
            .header("Idempotency-Key", &idempotency_key)
            .json(&payload)
            .send()
            .await 
    };
    tokio::pin!(req_a);

    // 6. Dynamic p95 Timeout calculation
    let p95_ms = get_p95_timeout(&state);
    let dynamic_timeout = std::time::Duration::from_millis(p95_ms);

    // 7. Hedged Request Execution with Dynamic Timeout
    let response = tokio::select! {
        res = &mut req_a => {
            tracing::debug!("Fast backend response. Request resolved without Hedge.");
            res
        },
        _ = tokio::time::sleep(dynamic_timeout) => {
            tracing::warn!("Tail latency detected (p95: {}ms). Firing Hedged Request...", p95_ms);
            
            let idx_b = (idx_a + 1) % total_replicas;
            let url_b = format!("{}/v1/chat/completions", state.backend_urls[idx_b]);
            tracing::warn!("Routing Hedged Request B to replica [{}]", url_b);

            let req_b = async { 
                client.post(&url_b)
                    .header("Idempotency-Key", &idempotency_key)
                    .json(&payload_clone)
                    .send()
                    .await 
            };
            tokio::pin!(req_b);

            tokio::select! {
                res_a = &mut req_a => res_a,
                res_b = &mut req_b => {
                    tracing::info!("The Hedged Request won the race and saved tail latency!");
                    res_b
                },
            }
        }
    };

    // 8. Handle connection Result and extract Status BEFORE consuming text
    let response = response.map_err(|e| {
        tracing::error!("Backend connection failed: {}", e);
        (axum::http::StatusCode::BAD_GATEWAY, "Failed to communicate with the model".to_string())
    })?;

    let status = response.status(); 

    // 9. Await text extraction (IO boundary)
    let response_text = response.text().await.map_err(|e| {
        tracing::error!("Failed to extract response text: {}", e);
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to extract text".to_string())
    })?;

    // 10. Record latency metrics synchronously AFTER all async IO is complete
    let duration = start.elapsed().as_millis() as u64;
    {
        let mut latencies = state.latencies.write().unwrap();
        if latencies.len() >= 100 { 
            latencies.pop_front(); 
        }
        latencies.push_back(duration);
    }

    // 11. Extract full_text for the Tokenizer
    let full_text = payload["messages"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join(" ");

    let text_to_encode = full_text.to_string();
    let prompt_tokens = state.tokenizer.encode(text_to_encode, false)
        .map(|encoded| encoded.get_ids().len())
        .unwrap_or(0);

    // 12. Cache Poisoning Shield (Only cache 200 OK)
    if status.is_success() {
        let cache_entry = CacheEntry {
            response_json: response_text.clone(),
            prompt_tokens,
        };
        
        if let Ok(entry_json) = serde_json::to_string(&cache_entry) {
            state.cache.insert(cache_key, entry_json).await;
        }
    } else {
        tracing::warn!("Skipping cache insertion due to non-200 status: {}", status);
    }

    // 13. Build and return the response
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("x-prompt-tokens", prompt_tokens.into());
    headers.insert(axum::http::header::CONTENT_TYPE, axum::http::HeaderValue::from_static("application/json"));

    Ok((status, headers, response_text).into_response())
}