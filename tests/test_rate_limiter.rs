// tests/test_rate_limiter.rs
mod common;

use axum::{body::Body, http::{Request, StatusCode}};
use serde_json::json;
use std::net::SocketAddr;
use tower::ServiceExt;
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

// 1. TEST: Fail-Open Behavior (Redis is Offline/None)
// This test uses our default setup_test_app where redis_client is None.
#[tokio::test]
async fn test_rate_limiter_fail_open_allows_unlimited_requests() {
    let mock_server = MockServer::start().await;
    
    // We expect 10 calls. If the rate limiter wasn't failing open, 
    // a low limit (like 5) would block some of these.
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl-mock",
            "choices": [{"message": {"content": "Fail open!"}}]
        })))
        .expect(10) 
        .mount(&mock_server)
        .await;

    let app = common::setup_test_app(mock_server.uri());

    // Send 10 rapid-fire requests. All should pass because Redis is None.
    for i in 0..10 {
        // Change the content slightly on each iteration to bypass the Moka Cache
        let payload = json!({
            "model": "llama2",
            "messages": [{"role": "user", "content": format!("Hello attempt {}", i)}]
        });

        let request = Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

// 2. TEST: IP Isolation and Actual Limit (Requires Local Redis)
// Run this specifically with: cargo test -- --ignored
#[tokio::test]
#[ignore = "Requires local Redis instance running on port 6379"]
async fn test_rate_limiter_enforces_limit_and_isolates_ips() {
    let mock_server = MockServer::start().await;
    
    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"content": "Rate limit test"}}]
        })))
        .mount(&mock_server)
        .await;

    // Usa a nossa nova função que liga o Redis!
    let app = common::setup_test_app_with_redis(mock_server.uri().to_string());
    
    let payload = json!({"model": "llama2", "messages": []});
    let mut hits = 0;

    // 1. Simula o Usuário A (IP: 192.168.1.100) batendo até tomar Rate Limit
    loop {
        let mut request = Request::builder()
            .uri("/v1/chat/completions")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap();
            
        // Injeta o IP falso do Usuário A nos extensions da requisição do Axum
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            SocketAddr::from(([192, 168, 1, 100], 8080))
        ));

        let response = app.clone().oneshot(request).await.unwrap();
        
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            tracing::info!("User A hit the rate limit after {} requests", hits);
            break; // Limite funcionou!
        }
        
        assert_eq!(response.status(), StatusCode::OK);
        hits += 1;
        
        if hits > 200 { 
            panic!("Rate limit was never reached! Check your Redis or middleware limit."); 
        }
    }

    // 2. Simula o Usuário B (IP: 10.0.0.5) logo em seguida.
    // Ele DEVE passar com 200 OK, provando o isolamento por IP!
    let mut request_b = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();
        
    // Injeta o IP falso do Usuário B
    request_b.extensions_mut().insert(axum::extract::ConnectInfo(
        SocketAddr::from(([10, 0, 0, 5], 9090))
    ));

    let response_b = app.clone().oneshot(request_b).await.unwrap();
    
    // Se o isolamento falhasse, o Usuário B tomaria 429 TOO_MANY_REQUESTS também.
    assert_eq!(response_b.status(), StatusCode::OK, "User B was unfairly rate limited!");
}