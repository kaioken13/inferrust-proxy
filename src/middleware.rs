// src/middleware.rs
use axum::{
    body::Body,
    extract::Request,
    http::{Response, StatusCode},
    response::IntoResponse,
};
use std::sync::Arc;
use tower::{Layer, Service};

use crate::rate_limiter::RateLimiter;

#[derive(Clone)]
pub struct RateLimitLayer {
    pub limiter: Option<Arc<dyn RateLimiter>>,
    pub limit: usize,
    pub window_secs: u64,
    pub behind_proxy: bool,
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitMiddleware {
            inner,
            limiter: self.limiter.clone(),
            limit: self.limit,
            window_secs: self.window_secs,
            behind_proxy: self.behind_proxy,
        }
    }
}

#[derive(Clone)]
pub struct RateLimitMiddleware<S> {
    inner: S,
    limiter: Option<Arc<dyn RateLimiter>>,
    limit: usize,
    window_secs: u64,
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
        let limiter_opt = self.limiter.clone();
        let limit = self.limit;
        let window_secs = self.window_secs;

        // Extrai o IP do cliente respeitando proxies upstream se configurado
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
            if let Some(limiter) = limiter_opt {
                match limiter.check(&client_ip, limit, window_secs).await {
                    Ok(allowed) => {
                        if !allowed {
                            return Ok(StatusCode::TOO_MANY_REQUESTS.into_response());
                        }
                    }
                    Err(e) => {
                        // Fail-open: falhas no rate limiter não derrubam o tráfego do gateway
                        tracing::error!("Rate limiter error: {}. Failing open.", e);
                    }
                }
            }

            inner.call(req).await
        })
    }
}
