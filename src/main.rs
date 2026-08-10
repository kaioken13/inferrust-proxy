use axum::{
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    // Inicializa o sistema de logs/tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "inferrust_proxy=debug,info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Roteamento da aplicação
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/v1/chat/completions", post(chat_completions_handler));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Proxy rodando em http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "inferrust-proxy" }))
}

async fn chat_completions_handler(Json(payload): Json<Value>) -> Json<Value> {
    tracing::info!("Requisição recebida em /v1/chat/completions");

    Json(json!({
        "id": "chatcmpl-proxy-mock",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "mock-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "InferRust-Proxy operacional. Aguardando integração do backend."
            },
            "finish_reason": "stop"
        }],
        "received_payload": payload
    }))
}