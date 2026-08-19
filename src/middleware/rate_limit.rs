use axum::{body::Body, extract::Request, http::{StatusCode, Response}, response::IntoResponse};
use std::sync::Arc;
use tower::{Layer, Service};
use redis::AsyncCommands;

#[derive(Clone)]
pub struct RateLimitLayer {
    pub redis_client: redis::Client,
    pub limit: i32,
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitMiddleware {
            inner,
            redis_client: self.redis_client.clone(),
            limit: self.limit,
        }
    }
}

#[derive(Clone)]
pub struct RateLimitMiddleware<S> {
    inner: S,
    redis_client: redis::Client,
    limit: i32,
}

impl<S> Service<Request> for RateLimitMiddleware<S>
where
    S: Service<Request, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = futures::future::BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_cx(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let mut inner = self.inner.clone();
        let client = self.redis_client.clone();
        let limit = self.limit;

        Box::pin(async move {
            let mut conn = client.get_async_connection().await.unwrap();
            let key = "rate_limit:global"; // Pode ser parametrizado por IP depois

            let count: i32 = conn.incr(&key, 1).await.unwrap_or(0);
            if count == 1 { conn.expire::<&str, i32>(&key, 60).await.ok(); }

            if count > limit {
                return Ok(StatusCode::TOO_MANY_REQUESTS.into_response());
            }

            inner.call(req).await
        })
    }
}