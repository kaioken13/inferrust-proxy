// src/middleware.rs
use axum::{
    body::Body,
    extract::Request,
    http::{Response, StatusCode},
    response::IntoResponse,
};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct RateLimitLayer {
    pub redis_client: Option<redis::Client>, // Optional Redis client for local mode support
    pub limit: i32,
    pub behind_proxy: bool,
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitMiddleware {
            inner,
            redis_client: self.redis_client.clone(),
            limit: self.limit,
            behind_proxy: self.behind_proxy,
        }
    }
}

#[derive(Clone)]
pub struct RateLimitMiddleware<S> {
    inner: S,
    redis_client: Option<redis::Client>,
    limit: i32,
    behind_proxy: bool,
}

impl<S> Service<Request> for RateLimitMiddleware<S>
where
    S: Service<Request, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = futures::future::BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let mut inner = self.inner.clone();
        let client_opt = self.redis_client.clone();
        let limit = self.limit;

        // Extract client IP to isolate rate-limiting per user and prevent global denial-of-service
        let client_ip = if self.behind_proxy {
            req.headers()
                .get("x-forwarded-for")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.split(',').next())
                .and_then(|ip_str| ip_str.trim().parse::<std::net::IpAddr>().ok())
                .map(|ip| ip.to_string())
                .or_else(|| {
                    req.headers()
                        .get("cf-connecting-ip")
                        .and_then(|h| h.to_str().ok())
                        .and_then(|ip_str| ip_str.trim().parse::<std::net::IpAddr>().ok())
                        .map(|ip| ip.to_string())
                })
        } else {
            None
        }
        .unwrap_or_else(|| {
            req.extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|addr| addr.0.ip().to_string())
                .unwrap_or_else(|| "anonymous".to_string())
        });

        Box::pin(async move {
            let client = match client_opt {
                Some(c) => c,
                None => return inner.call(req).await,
            };

            let mut conn = match client.get_multiplexed_async_connection().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(
                        "Redis connection failed (Fail-Open): {}. Passing through.",
                        e
                    );
                    return inner.call(req).await;
                }
            };

            let key = format!("rate_limit:{}", client_ip);

            // Atomic approach: Use a transaction to ensure INCR and EXPIRE are linked
            // or simply check if the key exists to set the TTL.
            let result: Result<(i32, ()), redis::RedisError> = redis::pipe()
                .atomic()
                .incr(&key, 1)
                .expire(&key, 60)
                .query_async(&mut conn)
                .await;

            let count = match result {
                Ok((val, _)) => val,
                Err(e) => {
                    tracing::error!(
                        "Failed to execute atomic rate limit in Redis: {}. Failing open.",
                        e
                    );
                    return inner.call(req).await;
                }
            };

            if count > limit {
                return Ok(StatusCode::TOO_MANY_REQUESTS.into_response());
            }

            inner.call(req).await
        })
    }
}
