// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rate limiting — in-memory per-key token bucket.
//!
//! Each bucket has a fixed `capacity` and a `refill_window` duration
//! over which the bucket linearly refills back to capacity. The
//! refill is computed lazily on each `consume` call, so there's no
//! background timer.
//!
//! Per-key sharding lives in [`PerKeyLimiter`] — a small concurrent
//! map keyed by client identity (typically `<ip, route>`) that
//! auto-creates buckets on first hit. Eviction happens lazily when a
//! bucket has been idle past one full window.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Single bucket. Thread-safe — `Mutex` is acquired only for the
/// duration of `consume`.
#[derive(Debug)]
pub struct TokenBucket {
    capacity: f64,
    refill_per_sec: f64,
    inner: Mutex<BucketInner>,
}

#[derive(Debug)]
struct BucketInner {
    available: f64,
    last_refill: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allow { remaining: u32 },
    Deny { retry_after_secs: u64 },
}

impl TokenBucket {
    /// Construct a bucket with `capacity` tokens that refills back to
    /// full over `refill_window`.
    pub fn new(capacity: u32, refill_window: Duration) -> Self {
        let cap = capacity as f64;
        let secs = refill_window.as_secs_f64().max(0.001);
        Self {
            capacity: cap,
            refill_per_sec: cap / secs,
            inner: Mutex::new(BucketInner {
                available: cap,
                last_refill: Instant::now(),
            }),
        }
    }

    pub fn capacity(&self) -> u32 { self.capacity as u32 }

    /// Attempt to consume `tokens`. Returns `Allow { remaining }` if
    /// enough tokens were available, else `Deny { retry_after_secs }`
    /// with the number of seconds the caller must wait before the
    /// bucket holds at least one token.
    pub fn consume(&self, tokens: u32) -> RateLimitDecision {
        let cost = tokens as f64;
        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(g.last_refill).as_secs_f64();
        g.available = (g.available + elapsed * self.refill_per_sec).min(self.capacity);
        g.last_refill = now;
        if g.available >= cost {
            g.available -= cost;
            RateLimitDecision::Allow {
                remaining: g.available.floor() as u32,
            }
        } else {
            let deficit = cost - g.available;
            let wait_secs = (deficit / self.refill_per_sec).ceil() as u64;
            RateLimitDecision::Deny {
                retry_after_secs: wait_secs.max(1),
            }
        }
    }
}

/// Per-key limiter — lazily creates a [`TokenBucket`] per key.
#[derive(Debug)]
pub struct PerKeyLimiter {
    capacity: u32,
    refill_window: Duration,
    buckets: Mutex<HashMap<String, std::sync::Arc<TokenBucket>>>,
}

impl PerKeyLimiter {
    pub fn new(capacity: u32, refill_window: Duration) -> Self {
        Self {
            capacity,
            refill_window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn check(&self, key: &str) -> RateLimitDecision {
        let bucket = {
            let mut g = self.buckets.lock().unwrap();
            g.entry(key.to_string())
                .or_insert_with(|| std::sync::Arc::new(TokenBucket::new(self.capacity, self.refill_window)))
                .clone()
        };
        bucket.consume(1)
    }

    pub fn tracked_keys(&self) -> usize {
        self.buckets.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_starts_full() {
        let b = TokenBucket::new(10, Duration::from_secs(60));
        assert_eq!(b.capacity(), 10);
        let dec = b.consume(1);
        assert!(matches!(dec, RateLimitDecision::Allow { remaining: 9 }));
    }

    #[test]
    fn bucket_denies_at_zero_with_retry_after() {
        let b = TokenBucket::new(1, Duration::from_secs(60));
        let _ = b.consume(1);
        match b.consume(1) {
            RateLimitDecision::Deny { retry_after_secs } => {
                assert!(retry_after_secs >= 1);
            }
            _ => panic!("expected deny"),
        }
    }

    #[test]
    fn per_key_limiter_tracks_distinct_keys() {
        let l = PerKeyLimiter::new(5, Duration::from_secs(60));
        let _ = l.check("a");
        let _ = l.check("b");
        let _ = l.check("a");
        assert_eq!(l.tracked_keys(), 2);
    }

    #[test]
    fn per_key_limits_are_independent() {
        let l = PerKeyLimiter::new(1, Duration::from_secs(60));
        assert!(matches!(l.check("ip-a"), RateLimitDecision::Allow { .. }));
        assert!(matches!(l.check("ip-b"), RateLimitDecision::Allow { .. }));
        assert!(matches!(l.check("ip-a"), RateLimitDecision::Deny { .. }));
        assert!(matches!(l.check("ip-b"), RateLimitDecision::Deny { .. }));
    }
}
