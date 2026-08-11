use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    http_client: Client,
    backend_url: String,
}

#[tokio::main]
async fn main() {
    // Inicializa o sistema de logs/tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "inferrust_proxy=debug,info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Cria a estrutura de estado compartilhado
    let state = Arc::new(AppState {
        http_client: Client::new(),
        backend_url: std::env::var("BACKEND_URL").unwrap_or_else(|_| "http://localhost:11434".to_string()),
    });

    // Roteamento da aplicação
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Proxy rodando em http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "inferrust-proxy" }))
}

async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tracing::info!("Requisição recebida em /v1/chat/completions");

    let target_url = format!("{}/v1/chat/completions", state.backend_url);

    // Envia o payload recebido diretamente para o Ollama
    let response = state
        .http_client
        .post(&target_url)
        .json(&payload)
        .send()
        .await
        .map_err(|err| {
            tracing::error!("Erro ao se comunicar com o backend de inferência: {:?}", err);
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": "Falha na comunicação com o backend de inferência.",
                        "type": "bad_gateway",
                        "details": err.to_string()
                    }
                })),
            )
        })?;

        // Lê a resposta JSON do Ollama

        let response_json = response.json::<Value>().await.map_err(|err| {
            tracing::error!("Erro ao deserializar a responder do backend: {:?}", err);
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": "Resposta inválida recebida dp backend de inferência.",
                        "type": "internal_error"
                    }
                }))
            )
        })?;

        Ok(Json(response_json))
}