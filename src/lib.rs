use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;

// Estado compartilhado público para que o main.rs e os testes possam acessar
#[derive(Clone)]
pub struct AppState {
    pub http_client: Client,
    pub backend_url: String,
}

// Construtor público do Router (usado pelo main e pelos testes)
pub fn create_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/v1/chat/completions", post(chat_completions_handler))
        .with_state(state)
}

pub async fn health_check() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "inferrust-proxy" }))
}

pub async fn chat_completions_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tracing::info!("Requisição recebida em /v1/chat/completions");

    let target_url = format!("{}/v1/chat/completions", state.backend_url);

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

    let response_json = response.json::<Value>().await.map_err(|err| {
        tracing::error!("Erro ao deserializar a resposta do backend: {:?}", err);
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {
                    "message": "Resposta inválida recebida do backend de inferência.",
                    "type": "internal_error"
                }
            })),
        )
    })?;

    Ok(Json(response_json))
}