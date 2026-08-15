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