// tests/test_proxy.rs
mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

// 1. TEST: Health Check
#[tokio::test]
async fn test_health_check_returns_ok() {
    let app = common::setup_test_app("http://localhost:11434".to_string());

    let request = Request::builder()
        .uri("/health")
        .method("GET")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Read the body and parse it as JSON
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    // The new assertions that match your refactored handler!
    assert_eq!(json["status"], "ok");
    assert_eq!(json["message"], "inferrust-proxy");
}

// 2. TEST: Proxy Forwards Payload Correctly
#[tokio::test]
async fn test_proxy_forwards_payload_to_backend() {
    let mock_server = wiremock::MockServer::start().await;
    let backend_url = mock_server.uri();

    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .and(wiremock::matchers::body_json(&json!({
            "model": "llama2",
            "messages": [{"role": "user", "content": "Hello"}]
        })))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmpl-mock",
                "choices": [{"message": {"content": "Hello from mock!"}}]
            })),
        )
        .mount(&mock_server)
        .await;

    // Use the wiremock URL here!
    let app = common::setup_test_app(backend_url);

    let payload = json!({
        "model": "llama2",
        "messages": [{"role": "user", "content": "Hello"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["choices"][0]["message"]["content"], "Hello from mock!");
}

// 3. TEST: Backend Offline Returns 502 Bad Gateway
#[tokio::test]
async fn test_backend_unreachable_returns_502() {
    // Inject a broken port to force the 502
    let app = common::setup_test_app("http://127.0.0.1:99999".to_string());

    let payload = json!({ "model": "llama2", "messages": [] });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();

    println!("DEBUG - ACTUAL ERROR JSON: {:#?}", json);

    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Failed to communicate"));
}

// 4. TEST: Invalid Payload (Malformed JSON)
#[tokio::test]
async fn test_invalid_json_returns_bad_request() {
    let app = common::setup_test_app("http://localhost:11434".to_string());

    let invalid_json = "{{{{{ invalid }}}}}";

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(invalid_json))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// 5. TEST: Proxy Forwards Headers to Backend
#[tokio::test]
async fn test_proxy_forwards_headers_to_backend() {
    let mock_server = wiremock::MockServer::start().await;
    let backend_url = mock_server.uri();

    // Backend mock: Ensures that the Authorization header reaches the backend
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .and(wiremock::matchers::header("Authorization", "Bearer my-secret-token"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmpl-header",
                "choices": [{"message": {"content": "Authorized!"}}]
            })),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let app = common::setup_test_app(backend_url);

    let payload = json!({
        "model": "llama2",
        "messages": [{"role": "user", "content": "Hello"}]
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/chat/completions")
                .method("POST")
                .header("content-type", "application/json")
                .header("Authorization", "Bearer my-secret-token") // Injecting the header
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}