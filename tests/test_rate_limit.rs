// tests/test_rate_limit.rs
mod common;

use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_rate_limiter_shapes_traffic() {
    let app = common::setup_test_app("http://localhost:11434".to_string());

    let start = std::time::Instant::now();

    let make_req = || {
        Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(r#"{"model": "test"}"#))
            .unwrap()
    };

    // Fire 3 requests concurrently.
    // Limit is 2 per second. The 3rd request will be held in the buffer!
    let (res1, res2, res3) = tokio::join!(
        app.clone().oneshot(make_req()),
        app.clone().oneshot(make_req()),
        app.clone().oneshot(make_req())
    );

    let elapsed = start.elapsed();

    // All of them should eventually succeed with a 200 OK
    assert_eq!(res1.unwrap().status(), StatusCode::OK);
    assert_eq!(res2.unwrap().status(), StatusCode::OK);
    assert_eq!(res3.unwrap().status(), StatusCode::OK);

    // The undeniable proof: The total execution MUST take at least 1 second
    assert!(
        elapsed.as_secs_f64() >= 1.0,
        "The rate limiter failed! It processed 3 requests in less than 1 second. Elapsed: {:?}",
        elapsed
    );
}