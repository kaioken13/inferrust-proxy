use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder,
};
use std::sync::OnceLock;

struct AppMetrics {
    registry: Registry,
    llm_time_to_first_token_seconds: HistogramVec,
    llm_inter_token_latency_seconds: HistogramVec,
    request_total: IntCounterVec,
}

fn get_metrics() -> &'static AppMetrics {
    static METRICS: OnceLock<AppMetrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        let registry = Registry::new();

        let llm_time_to_first_token_seconds = HistogramVec::new(
            HistogramOpts::new(
                "llm_time_to_first_token_seconds",
                "Time from request receipt until the first SSE data chunk arrives from the upstream provider",
            )
            .buckets(vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0]),
            &["model", "stream"],
        )
        .expect("valid ttft histogram");

        let llm_inter_token_latency_seconds = HistogramVec::new(
            HistogramOpts::new(
                "llm_inter_token_latency_seconds",
                "Time between consecutive streaming chunks (decode throughput)",
            )
            .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]),
            &["model"],
        )
        .expect("valid itl histogram");

        let request_total = IntCounterVec::new(
            Opts::new("request_total", "Total LLM proxy requests"),
            &["model", "status", "stream"],
        )
        .expect("valid request counter");

        registry
            .register(Box::new(llm_time_to_first_token_seconds.clone()))
            .expect("register ttft");
        registry
            .register(Box::new(llm_inter_token_latency_seconds.clone()))
            .expect("register itl");
        registry
            .register(Box::new(request_total.clone()))
            .expect("register request_total");

        AppMetrics {
            registry,
            llm_time_to_first_token_seconds,
            llm_inter_token_latency_seconds,
            request_total,
        }
    })
}

pub fn llm_time_to_first_token_seconds() -> &'static HistogramVec {
    &get_metrics().llm_time_to_first_token_seconds
}

pub fn llm_inter_token_latency_seconds() -> &'static HistogramVec {
    &get_metrics().llm_inter_token_latency_seconds
}

pub fn request_total() -> &'static IntCounterVec {
    &get_metrics().request_total
}

pub fn gather_metrics() -> String {
    let encoder = TextEncoder::new();
    let metric_families = get_metrics().registry.gather();
    let mut buffer = Vec::new();
    let _ = encoder.encode(&metric_families, &mut buffer);
    String::from_utf8(buffer).unwrap_or_default()
}
