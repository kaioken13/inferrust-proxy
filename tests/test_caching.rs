// tests/test_caching.rs
mod common;

use axum::{body::Body, http::Request, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::{ServiceExt};
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

/// Helper function exclusive to this test to avoid code repetition
async fn send_request(app: &Router, payload: &Value) -> Value {
    let request = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    // Unlike .oneshot() which consumes the route, .ready().call() allows 
    // us to reuse the same App instance multiple times!
    let response = app.clone().oneshot(request).await.unwrap();
    
    // Extract the body and convert it back to JSON for validation
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body_bytes).unwrap()
}

#[tokio::test]
async fn test_cache_returns_cached_response_on_second_request() {
    // 1. Set up the fake backend mock
    let mock_server = MockServer::start().await;
    let backend_url = mock_server.uri();

    // Backend mock: MUST be called EXACTLY ONCE
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmpl-123",
                "choices": [{"message": {"content": "Mock response"}}]
            })),
        )
        .expect(1) // Expect exactly one call to this mock
        .mount(&mock_server)
        .await;

    // 2. Create the App via common.rs
    let app = common::setup_test_app(backend_url);

    let payload = json!({
        "model": "llama2",
        "messages": [{"role": "user", "content": "Hi"}]
    });

    // 3. 1st request: Cache is empty, it must hit Wiremock (MISS)
    let response1 = send_request(&app, &payload).await;
    assert_eq!(response1["choices"][0]["message"]["content"], "Mock response");

    // 4. 2nd request: It must come directly from the cache (HIT) - WITHOUT hitting Wiremock
    let response2 = send_request(&app, &payload).await;
    assert_eq!(response2["choices"][0]["message"]["content"], "Mock response");

    // Wiremock magic: If the second request leaked to the network,
    // the mock_server would fail the test right now because we demanded .expect(1).
}

// 2. TEST: Cache Poisoning Prevention (Different parameters = Different Cache Keys)
#[tokio::test]
async fn test_cache_poisoning_prevention_with_different_temperature() {
    let mock_server = MockServer::start().await;
    let backend_url = mock_server.uri();

    // Backend mock MUST be called EXACTLY TWICE, because the payloads have different temperatures.
    // If cache poisoning was occurring, it would only be called once.
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmpl-diff",
                "choices": [{"message": {"content": "Response"}}]
            })),
        )
        .expect(2) // We expect 2 network calls (2 cache misses)
        .mount(&mock_server)
        .await;

    let app = common::setup_test_app(backend_url);

    // Payload 1: Temperature 1.0
    let payload1 = json!({
        "model": "llama2",
        "messages": [{"role": "user", "content": "Explain Rust"}],
        "temperature": 1.0
    });

    // Payload 2: Temperature 0.0 (Same exact message, different temperature)
    let payload2 = json!({
        "model": "llama2",
        "messages": [{"role": "user", "content": "Explain Rust"}],
        "temperature": 0.0
    });

    // 1st request: Miss (Hits Wiremock)
    let response1 = send_request(&app, &payload1).await;
    assert_eq!(response1["id"], "cmpl-diff");

    // 2nd request: Miss again (Hits Wiremock) because of different temperature
    let response2 = send_request(&app, &payload2).await;
    assert_eq!(response2["id"], "cmpl-diff");
}

// 3. TEST: Streaming Cache Hit (Advanced SSE Caching)
#[tokio::test]
async fn test_cache_returns_cached_sse_for_streaming_requests() {
    let mock_server = wiremock::MockServer::start().await;
    let backend_url = mock_server.uri();

    // Backend mock MUST be called EXACTLY ONCE!
    // The second request must be served directly from your SSE Cache.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("data: {\"choices\": [{\"delta\": {\"content\": \"Stream \"}}]}\n\ndata: [DONE]\n\n"),
        )
        .expect(1) // <-- A magia acontece aqui: só 1 chamada real na rede!
        .mount(&mock_server)
        .await;

    let app = common::setup_test_app(backend_url);

    let payload = serde_json::json!({
        "model": "llama2",
        "messages": [{"role": "user", "content": "Stream this"}],
        "stream": true
    });

    // Helper closure to build the request since we can't use `send_request` for SSE
    let build_req = || {
        axum::http::Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap()
    };

    // 1st request: Miss (Hits MockServer and gets cached by your Write-Behind logic)
    let response1 = app.clone().oneshot(build_req()).await.unwrap();
    assert_eq!(response1.status(), axum::http::StatusCode::OK);
    
    // Consume the body to complete the request
    let _ = http_body_util::BodyExt::collect(response1.into_body()).await.unwrap().to_bytes();

    // 2nd request: HIT (Served from Cache with SSE headers)
    let response2 = app.clone().oneshot(build_req()).await.unwrap();
    assert_eq!(response2.status(), axum::http::StatusCode::OK);

    // Validate if your proxy correctly restored the SSE headers from the cache
    let content_type = response2.headers().get("content-type").unwrap().to_str().unwrap();
    assert_eq!(content_type, "text/event-stream");

    let cache_control = response2.headers().get("cache-control").unwrap().to_str().unwrap();
    assert_eq!(cache_control, "no-cache");

    // Validate the body content
    let body_bytes = http_body_util::BodyExt::collect(response2.into_body()).await.unwrap().to_bytes();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Stream "));
    assert!(body_str.contains("[DONE]"));
}

// 4. TEST: Cache Expiration (TTL)
#[tokio::test]
async fn test_cache_expires_after_ttl() {
    let mock_server = MockServer::start().await;
    let backend_url = mock_server.uri();

    // Backend mock MUST be called EXACTLY TWICE.
    // 1st time: Initial miss.
    // 2nd time: Miss after the cache has expired.
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmpl-ttl",
                "choices": [{"message": {"content": "TTL Response"}}]
            })),
        )
        .expect(2) // We expect exactly 2 network calls
        .mount(&mock_server)
        .await;

    // Use our new helper to set a ridiculously short TTL: 2 seconds!
    let app = common::setup_test_app_with_ttl(backend_url, 2);

    let payload = json!({
        "model": "llama2",
        "messages": [{"role": "user", "content": "Testing TTL"}]
    });

    // 1st request: Miss (Hits Wiremock)
    let response1 = send_request(&app, &payload).await;
    assert_eq!(response1["id"], "cmpl-ttl");

    // 2nd request immediately: HIT (Does NOT hit Wiremock)
    let response2 = send_request(&app, &payload).await;
    assert_eq!(response2["id"], "cmpl-ttl");
    
    tracing::info!("Sleeping for 3 seconds to let the cache expire...");
    
    // Simulate time passing: Sleep for 3 seconds (1 second longer than our TTL)
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // 3rd request after TTL: Miss again (Hits Wiremock and completes the .expect(2) requirement)
    let response3 = send_request(&app, &payload).await;
    assert_eq!(response3["id"], "cmpl-ttl");
}