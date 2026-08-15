// tests/test_rate_limit.rs
mod common;

use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use wiremock::{matchers, Mock, MockServer, ResponseTemplate}; // 👈 Added wiremock

#[tokio::test]
async fn test_rate_limiter_shapes_traffic() {
    // 1. Spin up a fake backend that ALWAYS returns 200 OK
    let mock_server = MockServer::start().await;
    Mock::given(matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&mock_server)
        .await;

    // 2. Point the proxy to the fake Wiremock backend
    let app = common::setup_test_app(mock_server.uri());

    let start = std::time::Instant::now();

    // 3. Send a properly formatted payload
    let payload = serde_json::json!({
        "model": "test",
        "messages": [{"role": "user", "content": "hi"}]
    });

    let make_req = || {
        Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(payload.to_string()))
            .unwrap()
    };

    // 4. Fire 3 requests concurrently
    let (res1, res2, res3) = tokio::join!(
        app.clone().oneshot(make_req()),
        app.clone().oneshot(make_req()),
        app.clone().oneshot(make_req())
    );

    let elapsed = start.elapsed();

    // Now they will definitively return 200 OK!
    assert_eq!(res1.unwrap().status(), StatusCode::OK);
    assert_eq!(res2.unwrap().status(), StatusCode::OK);
    assert_eq!(res3.unwrap().status(), StatusCode::OK);

    // Assert the traffic was delayed by at least 1 second
    assert!(
        elapsed.as_secs_f64() >= 1.0,
        "The rate limiter failed! Elapsed: {:?}",
        elapsed
    );
}