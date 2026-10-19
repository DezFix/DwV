use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct SpeedLimiter {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    rate: AtomicU64,
    bucket: Mutex<Bucket>,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

impl SpeedLimiter {
    pub fn new(limit: Option<u64>) -> Self {
        Self {
            inner: Arc::new(Inner {
                rate: AtomicU64::new(limit.unwrap_or(0)),
                bucket: Mutex::new(Bucket {
                    tokens: 0.0,
                    last: Instant::now(),
                }),
            }),
        }
    }

    pub fn set_limit(&self, limit: Option<u64>) {
        self.inner.rate.store(limit.unwrap_or(0), Ordering::Relaxed);
    }

    pub fn limit(&self) -> Option<u64> {
        match self.inner.rate.load(Ordering::Relaxed) {
            0 => None,
            n => Some(n),
        }
    }

    pub async fn acquire(&self, bytes: u64) {
        let bytes = bytes as f64;
        loop {
            let rate = self.inner.rate.load(Ordering::Relaxed);
            if rate == 0 {
                return;
            }
            let wait = {
                let mut bucket = self.inner.bucket.lock().unwrap();
                let now = Instant::now();
                let elapsed = now.duration_since(bucket.last).as_secs_f64();
                bucket.last = now;
                let cap = (rate as f64).max(bytes);
                bucket.tokens = (bucket.tokens + rate as f64 * elapsed).min(cap);
                if bucket.tokens >= bytes {
                    bucket.tokens -= bytes;
                    return;
                }
                (bytes - bucket.tokens) / rate as f64
            };
            // wake up to re-check in case the limit changes while sleeping
            tokio::time::sleep(Duration::from_secs_f64(wait.min(0.5))).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unlimited_returns_immediately() {
        let limiter = SpeedLimiter::new(None);
        let start = Instant::now();
        limiter.acquire(1024 * 1024).await;
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn limit_delays_transfers() {
        let limiter = SpeedLimiter::new(Some(128 * 1024));
        let start = Instant::now();
        for _ in 0..8 {
            limiter.acquire(16 * 1024).await;
        }
        // 128 KiB at 128 KiB/s takes about one second
        assert!(start.elapsed() >= Duration::from_millis(700));
    }
}
