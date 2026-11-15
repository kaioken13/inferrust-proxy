use super::{RateLimitError, RateLimiter};
use async_trait::async_trait;

#[derive(Clone)]
pub struct RedisRateLimiter {
    client: redis::Client,
}

impl RedisRateLimiter {
    pub fn new(client: redis::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl RateLimiter for RedisRateLimiter {
    async fn check(
        &self,
        key: &str,
        limit: usize,
        window_secs: u64,
    ) -> Result<bool, RateLimitError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| RateLimitError::Backend(e.to_string()))?;

        let now_sec = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let current_epoch = (now_sec / window_secs) as u32;
        // The key changes every 60s window: e.g., "rate_limit:127.0.0.1:28392100"
        let redis_key = format!("rate_limit:{}:{}", key, current_epoch);

        let result: Result<(i64, ()), redis::RedisError> = redis::pipe()
            .atomic()
            .incr(&redis_key, 1)
            .expire(&redis_key, (window_secs * 2) as i64) // Safety TTL for self-cleaning
            .query_async(&mut conn)
            .await;

        match result {
            Ok((count, _)) => Ok((count as usize) <= limit),
            Err(e) => Err(RateLimitError::Backend(e.to_string())),
        }
    }
}
