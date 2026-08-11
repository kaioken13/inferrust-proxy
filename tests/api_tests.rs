use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt; // Para o .collect().await no Axum 0.7
use inferrust_proxy::{create_app, AppState};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

// 1. TESTE: Health Check
#[tokio::test]
async fn test_health_check_returns_ok() {
    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_url: "http://localhost:11434".to_string(),
    });
    let app = create_app(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(body_json["status"], "ok");
    assert_eq!(body_json["service"], "inferrust-proxy");
}

// 2. TESTE: Proxy Encaminha Payload Corretamente
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

    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_url: backend_url.to_string(),
    });
    let app = create_app(state);

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

// 3. TESTE: Backend Offline Retorna 502 Bad Gateway
#[tokio::test]
async fn test_backend_unreachable_returns_502() {
    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_url: "http://127.0.0.1:59999".to_string(),
    });
    let app = create_app(state);

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
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Falha na comunicação"));
}

// 4. TESTE: Payload Inválido (JSON Malformado)
#[tokio::test]
async fn test_invalid_json_returns_bad_request() {
    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_url: "http://localhost:11434".to_string(),
    });
    let app = create_app(state);

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