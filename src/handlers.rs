// src/handlers.rs
use axum::http::header;
use axum::{extract::Request, response::Response};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use metrics::histogram;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{vectordb_client::SearchRequest, AppState};

use crate::cache::generate_cache_key;
use futures::StreamExt;

use crate::utils::get_p95_timeout;

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

#[derive(Deserialize)]
pub struct VectorSearchPayload {
    pub vector: Vec<f32>,
    pub k: u32,
}

#[derive(Serialize)]
pub struct VectorSearchResult {
    pub matches: Vec<Match>,
}

#[derive(Serialize)]
pub struct Match {
    pub id: u64,
    pub distance: f32,
}

pub async fn health_handler() -> impl IntoResponse {
    let health_status = serde_json::json!({
        "status": "ok",
        "message": "inferrust-proxy",
    });
    (StatusCode::OK, Json(health_status))
}

pub async fn forward_to_disputeguard(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Result<Response, StatusCode> {
    let (parts, body) = req.into_parts();

    // 1. Extração da Chave de Idempotência
    let idempotency_key = parts
        .headers
        .get("idempotency-key")
        .and_then(|val| val.to_str().ok())
        .map(|s| format!("idempotency:{}", s));

    // 2. Consulta no Redis se a chave foi fornecida
    if let (Some(ref key), Some(ref client)) = (&idempotency_key, &state.redis_client) {
        if let Ok(mut con) = client.get_multiplexed_async_connection().await {
            let cached: redis::RedisResult<Option<String>> = con.get(key).await;
            if let Ok(Some(cached_json)) = cached {
                tracing::info!("Idempotency HIT for key: {}", key);
                return Ok((
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "application/json"),
                        (header::HeaderName::from_static("x-cache"), "HIT"),
                    ],
                    cached_json,
                )
                    .into_response());
            }
        }
    }

    // 3. Lê o corpo da requisição para encaminhar ao DisputeGuard
    let bytes = axum::body::to_bytes(body, 10 * 1024 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let target_url = "http://disputeguard-api:8000/v1/disputes/resolve";
    let mut upstream_req = state.http_client.post(target_url).body(bytes);

    if let Some(content_type) = parts.headers.get(header::CONTENT_TYPE) {
        upstream_req = upstream_req.header(header::CONTENT_TYPE, content_type);
    }

    // 4. Executa a requisição upstream
    let upstream_res = upstream_req
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let status = upstream_res.status();
    let res_bytes = upstream_res
        .bytes()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    // 5. Se foi sucesso e há chave de idempotência, persiste no Redis (TTL = 86400s / 24h)
    if status.is_success() {
        if let (Some(ref key), Some(ref client)) = (&idempotency_key, &state.redis_client) {
            if let Ok(mut con) = client.get_multiplexed_async_connection().await {
                let body_str = String::from_utf8_lossy(&res_bytes).to_string();
                let _: redis::RedisResult<()> = con.set_ex(key, body_str, 86400).await;
                tracing::info!("Idempotency SET for key: {}", key);
            }
        }
    }

    Ok((
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::HeaderName::from_static("x-cache"), "MISS"),
        ],
        res_bytes,
    )
        .into_response())
}

pub async fn search_vectors(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<VectorSearchPayload>,
) -> Result<Json<VectorSearchResult>, StatusCode> {
    // 0. Extrai o client do Option; devolve 503 se o VectorDB estiver desligado
    let mut client = match state.vectordb_client.clone() {
        Some(client) => client,
        None => {
            tracing::warn!("Vector search requested but VectorDB integration is disabled");
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    // 1. Constructs the request aligned with the updated proto (query_vector)
    let request = tonic::Request::new(SearchRequest {
        query_vector: payload.vector,
        k: payload.k,
    });

    // 2. Call the search_nearest RPC exposed by vectordb-core
    let response = match client.search_nearest(request).await {
        Ok(res) => res.into_inner(),
        Err(status) => {
            tracing::error!("VectorDB gRPC search failed: {:?}", status);
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    // 3. Maps directly to the repeated SearchMatch matches
    let matches = response
        .matches
        .into_iter()
        .map(|m| Match {
            id: m.id,
            distance: m.distance,
        })
        .collect();

    Ok(Json(VectorSearchResult { matches }))
}

pub async fn chat_completions_handler(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<AppState>>,
    headers: axum::http::HeaderMap,
    axum::Json(payload): axum::extract::Json<serde_json::Value>,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    // 1. Capture start time for dynamic p95 calculation
    let start = std::time::Instant::now();

    // 2. Extract Authorization header if present
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string());

    // 3. Generate Cache Key using robust key generator
    let cache_key = generate_cache_key(&payload); // Adjust path if needed
    let is_stream = payload["stream"].as_bool().unwrap_or(false);

    // 4. FAST PATH: Cache HIT (Handles both Stream and Non-Stream)
    if let Some(cached_data) = state.cache.get(&cache_key).await {
        if let Ok(entry) = serde_json::from_str::<CacheEntry>(&cached_data) {
            tracing::info!(key = %cache_key, "Cache HIT");

            let mut res_headers = axum::http::HeaderMap::new();
            res_headers.insert("x-prompt-tokens", entry.prompt_tokens.into());

            if is_stream {
                res_headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("text/event-stream"),
                );
                res_headers.insert(
                    axum::http::header::CACHE_CONTROL,
                    axum::http::HeaderValue::from_static("no-cache"),
                );
                res_headers.insert(
                    axum::http::header::CONNECTION,
                    axum::http::HeaderValue::from_static("keep-alive"),
                );
            } else {
                res_headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/json"),
                );
            }

            return Ok((res_headers, axum::body::Body::from(entry.response_json)).into_response());
        }
    }

    tracing::info!(key = %cache_key, "Cache MISS. Processing request...");

    // Zero-heap-allocation, accurate ChatML token estimation
    let prompt_tokens = if let Some(messages) = payload["messages"].as_array() {
        let mut total_tokens = 0;

        for message in messages {
            // Every message follows <|im_start|>{role/name}\n{content}<|im_end|>\n
            // Base metadata framing overhead per message is ~3 tokens
            total_tokens += 3;

            // Encode content directly from slice &str (zero string clones/allocations)
            if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                if let Ok(encoded) = state.tokenizer.encode(content, false) {
                    total_tokens += encoded.get_ids().len();
                }
            }

            // Optional 'name' field tokens and framing if specified
            if let Some(name) = message.get("name").and_then(|n| n.as_str()) {
                if let Ok(encoded) = state.tokenizer.encode(name, false) {
                    total_tokens += encoded.get_ids().len();
                    total_tokens += 1; // Additional token overhead for the name field
                }
            }
        }

        // Add 3 tokens for the assistant priming prompt: <|im_start|>assistant\n
        total_tokens += 3;
        total_tokens
    } else {
        0
    };

    // 5. Prepare Idempotency, Dynamic p95 Timeout, and Load Balancing
    let idempotency_key = uuid::Uuid::new_v4().to_string();
    let total_replicas = state.backend_urls.len();
    let current_idx = state
        .next_replica
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let idx_a = current_idx % total_replicas;
    let url_a = format!("{}/v1/chat/completions", state.backend_urls[idx_a]);

    let client = state.http_client.clone();
    let payload_clone = payload.clone();
    let p95_ms = get_p95_timeout(&state);
    let dynamic_timeout = std::time::Duration::from_millis(p95_ms);

    // 6. Build Request A
    let req_a = async {
        let mut builder = client
            .post(&url_a)
            .header("Idempotency-Key", &idempotency_key)
            .json(&payload);

        if let Some(auth) = &auth_header {
            builder = builder.header("Authorization", auth);
        }
        builder.send().await
    };
    tokio::pin!(req_a);

    // 7. Execute Request (With Hedging if applicable)
    let response = if total_replicas <= 1 {
        let mut builder = client
            .post(&url_a)
            .header("Idempotency-Key", &idempotency_key)
            .json(&payload);

        if let Some(auth) = &auth_header {
            builder = builder.header("Authorization", auth);
        }

        builder.send().await.map_err(|e| {
            let err_json =
                serde_json::json!({"error": {"message": format!("Failed to communicate: {}", e)}});
            (axum::http::StatusCode::BAD_GATEWAY, err_json.to_string())
        })?
    } else {
        let res = tokio::select! {
            res = &mut req_a => {
                tracing::debug!("Fast response. Resolved without Hedge.");
                res
            },
            _ = tokio::time::sleep(dynamic_timeout) => {
                tracing::warn!("Tail latency detected. Firing Hedged Request...");
                let idx_b = (idx_a + 1) % total_replicas;
                let url_b = format!("{}/v1/chat/completions", state.backend_urls[idx_b]);

                let req_b = async {
                    let mut builder = client.post(&url_b)
                        .header("Idempotency-Key", &idempotency_key)
                        .json(&payload_clone);
                    if let Some(auth) = &auth_header {
                        builder = builder.header("Authorization", auth);
                    }
                    builder.send().await
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

        res.map_err(|e| {
            let err_json =
                serde_json::json!({"error": {"message": format!("Failed to communicate: {}", e)}});
            (axum::http::StatusCode::BAD_GATEWAY, err_json.to_string())
        })?
    };

    let status = response.status();

    // 8. Record TTFT metrics synchronously
    let duration = start.elapsed().as_millis() as u64;
    {
        let mut latencies = state.latencies.write().unwrap();
        if latencies.len() >= 100 {
            latencies.pop_front();
        }
        latencies.push_back(duration);
    }

    // 9. Process the Response based on Stream or Non-Stream Mode
    if is_stream {
        // --- STREAMING MODE (SSE) ---
        let mut res_headers = axum::http::HeaderMap::new();
        res_headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        res_headers.insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        );
        res_headers.insert(
            axum::http::header::CONNECTION,
            axum::http::HeaderValue::from_static("keep-alive"),
        );

        let mut stream = response.bytes_stream();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, reqwest::Error>>(128);

        let state_clone = state.clone();
        let cache_key_clone = cache_key.clone();
        let prompt_tokens_clone = prompt_tokens;
        let req_start = start;

        // Background worker: mede TTFT/ITL, repassa chunks para o cliente e salva no cache L1
        tokio::spawn(async move {
            let mut cache_buffer = Vec::new();
            let mut is_first_chunk = true;
            let mut last_chunk_time = std::time::Instant::now();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        let now = std::time::Instant::now();

                        if is_first_chunk {
                            // TTFT real: intervalo desde a chegada do request no proxy até o 1º chunk emitido
                            let ttft = req_start.elapsed().as_secs_f64();
                            histogram!("llm_time_to_first_token_seconds").record(ttft);
                            is_first_chunk = false;
                        } else {
                            // ITL real: latência decorrida entre chunks sucessivos de decode
                            let itl = now.duration_since(last_chunk_time).as_secs_f64();
                            histogram!("llm_inter_token_latency_seconds").record(itl);
                        }
                        last_chunk_time = now;

                        cache_buffer.extend_from_slice(&chunk);
                        if tx.send(Ok(chunk)).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Stream error while reading chunk: {}", e);
                        let _ = tx.send(Err(e)).await;
                        break;
                    }
                }
            }

            // Persistência no cache L1 Moka (apenas para status 2xx)
            if !cache_buffer.is_empty() && status.is_success() {
                if let Ok(full_sse_text) = String::from_utf8(cache_buffer) {
                    let cache_entry = CacheEntry {
                        response_json: full_sse_text,
                        prompt_tokens: prompt_tokens_clone,
                    };
                    if let Ok(entry_json) = serde_json::to_string(&cache_entry) {
                        state_clone.cache.insert(cache_key_clone, entry_json).await;
                    }
                }
            }
        });

        let body = axum::body::Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
        Ok((status, res_headers, body).into_response())
    } else {
        // --- NON-STREAMING MODE ---
        let response_text = response.text().await.map_err(|e| {
            tracing::error!("Failed to extract response text: {}", e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to extract text".to_string(),
            )
        })?;

        // Cache Poisoning Shield (Only cache 200 OK)
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

        let mut res_headers = axum::http::HeaderMap::new();
        res_headers.insert("x-prompt-tokens", prompt_tokens.into());
        res_headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );

        Ok((status, res_headers, response_text).into_response())
    }
}
