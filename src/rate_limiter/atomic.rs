use super::{RateLimitError, RateLimiter};
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct AtomicRateLimiter {
    windows: DashMap<String, Arc<AtomicU64>>,
}

impl Default for AtomicRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicRateLimiter {
    pub fn new() -> Self {
        Self {
            windows: DashMap::new(),
        }
    }

    #[inline]
    fn pack(epoch: u32, count: u32) -> u64 {
        ((epoch as u64) << 32) | (count as u64)
    }

    #[inline]
    fn unpack(val: u64) -> (u32, u32) {
        ((val >> 32) as u32, val as u32)
    }
}

#[async_trait]
impl RateLimiter for AtomicRateLimiter {
    async fn check(
        &self,
        key: &str,
        limit: usize,
        window_secs: u64,
    ) -> Result<bool, RateLimitError> {
        let now_sec = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let current_epoch = (now_sec / window_secs) as u32;

        let entry = self
            .windows
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(Self::pack(current_epoch, 0))));

        let state_atomic = entry.value().clone();

        loop {
            let current = state_atomic.load(Ordering::Acquire);
            let (epoch, count) = Self::unpack(current);

            let (next_epoch, next_count) = if epoch == current_epoch {
                (epoch, count.saturating_add(1))
            } else {
                // Nova janela de tempo iniciada: reseta contador
                (current_epoch, 1)
            };

            let next = Self::pack(next_epoch, next_count);

            match state_atomic.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok((next_count as usize) <= limit),
                Err(_) => {
                    // Contenção detectada (outra thread alterou o u64), repete o loop imediatamente
                    std::hint::spin_loop();
                }
            }
        }
    }
}
