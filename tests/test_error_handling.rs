// tests/test_error_handling.rs
mod common;

use axum::{body::Body, http::{Request, StatusCode}};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

// 1. TEST: 400 Bad Request (Malformed JSON payload)
#[tokio::test]
async fn test_error_400_on_malformed_json() {
    let app = common::setup_test_app("http://localhost:11434".to_string());

    // Send broken JSON (missing quotes, unclosed braces)
    let invalid_payload = "{ model: llama2, messages: [ }";

    let request = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(invalid_payload))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // The Axum JSON extractor should automatically reject this with a 400
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// 2. TEST: 404 Not Found (Invalid Route)
#[tokio::test]
async fn test_error_404_on_invalid_route() {
    let app = common::setup_test_app("http://localhost:11434".to_string());

    // Try to access a route that doesn't exist
    let request = Request::builder()
        .uri("/v1/chat/completions/invalid_endpoint")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// 3. TEST: 502 Bad Gateway (Backend Unreachable Format Check)
// We test this in proxy_tests too, but here we strictly validate the JSON error structure.
#[tokio::test]
async fn test_error_502_returns_structured_json() {
    // Inject a dummy/dead port
    let app = common::setup_test_app("http://127.0.0.1:55555".to_string());

    let payload = json!({
        "model": "llama2",
        "messages": [{"role": "user", "content": "Hello"}]
    });

    let request = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Check Status
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    // Validate if the Error is structured as valid JSON
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json_error: Value = serde_json::from_slice(&body_bytes).expect("Error response is not valid JSON!");

    // It MUST contain the "error" -> "message" format we implemented in handlers.rs
    let error_message = json_error["error"]["message"].as_str().expect("Missing error.message in JSON");
    assert!(error_message.contains("Failed to communicate"));
}

// 4. TEST: 429 Too Many Requests (Rate Limit Response)
// Use `cargo test -- --ignored` when Redis is running
#[tokio::test]
#[ignore = "Requires local Redis instance running on port 6379"]
async fn test_error_429_on_rate_limit_exceeded() {
    let mock_server = wiremock::MockServer::start().await;
    
    // The mock server doesn't need to expect a specific number of calls here,
    // we just need it to respond so the proxy doesn't fail with 502 before hitting 429.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&mock_server)
        .await;

    // We MUST use the Redis-enabled setup to trigger a real 429
    let app = common::setup_test_app_with_redis(mock_server.uri());

    let payload = json!({
        "model": "llama2",
        "messages": []
    });

    let mut hit_429 = false;

    // Bombard the server to force the 429 error
    for _ in 0..200 { // Adjust this number if your limit is higher
        let mut request = Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap();

        // Inject a dedicated IP specifically for the Error Handling test
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            std::net::SocketAddr::from(([8, 8, 8, 8], 80))
        ));

        let response = app.clone().oneshot(request).await.unwrap();

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            hit_429 = true;
            
            // Note: In your middleware.rs, you currently return:
            // Ok(StatusCode::TOO_MANY_REQUESTS.into_response())
            // Which means the body is empty. If you ever change it to return a JSON error 
            // like the 502, you would assert that JSON structure right here!
            
            break;
        }
    }

    assert!(hit_429, "Expected the proxy to eventually return a 429 Too Many Requests error");
}