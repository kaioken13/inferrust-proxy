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

        let redis_key = format!("rate_limit:{}", key);

        let result: Result<(i64, ()), redis::RedisError> = redis::pipe()
            .atomic()
            .incr(&redis_key, 1)
            .expire(&redis_key, window_secs as i64)
            .query_async(&mut conn)
            .await;

        match result {
            Ok((count, _)) => Ok((count as usize) <= limit),
            Err(e) => Err(RateLimitError::Backend(e.to_string())),
        }
    }
}
