use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use inferrust_proxy::rate_limiter::{
    atomic::AtomicRateLimiter, redis_backend::RedisRateLimiter, RateLimiter,
};
use std::sync::Arc;
use tokio::runtime::Runtime;

fn bench_rate_limiters(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Setup: Inicializa os dois limiters
    let atomic_limiter = Arc::new(AtomicRateLimiter::new());

    // Redis local (certifique-se de que o container do Redis esteja acessível se rodar o grupo Redis)
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let redis_client = redis::Client::open(redis_url).ok();

    let mut group = c.benchmark_group("RateLimiter_Contention_SameKey");

    // Benchmark 1: AtomicRateLimiter sob contenção na mesma chave
    group.bench_function("Atomic_SameKey_CAS", |b| {
        let limiter = atomic_limiter.clone();
        b.to_async(&rt).iter(|| async {
            let allowed = limiter.check(black_box("192.168.1.100"), 100_000, 60).await;
            black_box(allowed)
        });
    });

    // Benchmark 2: RedisRateLimiter sob a mesma chave (se Redis disponível)
    if let Some(client) = redis_client {
        let redis_limiter = Arc::new(RedisRateLimiter::new(client));
        group.bench_function("Redis_SameKey_Pipeline", |b| {
            let limiter = redis_limiter.clone();
            b.to_async(&rt).iter(|| async {
                let allowed = limiter.check(black_box("192.168.1.100"), 100_000, 60).await;
                black_box(allowed)
            });
        });
    }

    group.finish();

    // Benchmark 3: Alta cardinalidade no AtomicRateLimiter
    let mut group_keys = c.benchmark_group("RateLimiter_DistinctKeys");
    group_keys.bench_function("Atomic_DistinctKeys_DashMap", |b| {
        let limiter = atomic_limiter.clone();
        let mut counter: usize = 0;
        b.to_async(&rt).iter(|| {
            counter = counter.wrapping_add(1);
            let key = format!("10.0.{}.{}", (counter >> 8) & 0xFF, counter & 0xFF);
            let limiter = limiter.clone();
            async move {
                let allowed = limiter.check(&key, 100_000, 60).await;
                black_box(allowed)
            }
        });
    });
    group_keys.finish();
}

criterion_group!(benches, bench_rate_limiters);
criterion_main!(benches);
