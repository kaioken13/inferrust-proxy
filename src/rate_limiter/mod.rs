pub mod atomic;
pub mod redis_backend;

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("Backend communication error: {0}")]
    Backend(String),
}

#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Retorna Ok(true) se a requisição está dentro do limite,
    /// Ok(false) se deve ser barrada (HTTP 429).
    async fn check(
        &self,
        key: &str,
        limit: usize,
        window_secs: u64,
    ) -> Result<bool, RateLimitError>;
}
