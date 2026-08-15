// tests/test_tokenizer.rs
mod common;

use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_proxy_injects_token_count_header() {
    // 1. Start the fake backend so we don't rely on a real LLM
    let mock_server = MockServer::start().await;
    Mock::given(matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"status": "success"}"#))
        .mount(&mock_server)
        .await;

    // 2. Setup the proxy app
    let app = common::setup_test_app(mock_server.uri());

    // 3. Create a payload with text
    let payload = serde_json::json!({
        "model": "test",
        "messages": [{"role": "user", "content": "Hello, how are you today?"}]
    });

    let request = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(payload.to_string()))
        .unwrap();

    // 4. Send the request
    let response = app.oneshot(request).await.unwrap();

    // 5. Assert the status is 200 OK
    assert_eq!(response.status(), StatusCode::OK);

    // 6. Assert the header exists!
    let token_header = response.headers().get("x-prompt-tokens");
    assert!(token_header.is_some(), "The x-prompt-tokens header is missing!");

    // 7. Verify the header is a valid number greater than 0
    let token_value = token_header.unwrap().to_str().unwrap();
    let token_count: usize = token_value.parse().unwrap();
    
    println!("✅ Test extracted {} tokens from the header!", token_count);
    assert!(token_count > 0, "Token count should be greater than zero");
}