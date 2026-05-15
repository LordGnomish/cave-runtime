//! Per-consumer rate limiting — request bucket + cost-weighted token bucket.
//!
//! **Sweep-006 adoption (2026-05-12)** — the bucket primitive is now
//! `cave_kernel::ratelimiter::TokenBucket`. Cost-weighted consumption is
//! supported natively (kernel's `try_consume(n)` accepts any positive
//! amount); the deficit-to-retry-after conversion that used to live here
//! has moved to `try_consume_or_retry_at`, which returns `Result<(), Duration>`.
//!
//! Per-consumer customisation (the `set_limit` / `get_limit` surface) is
//! still done locally because each consumer may have a different
//! capacity/refill pair — kernel's `PerTenant<B>` only supports a single
//! factory, so we keep a `DashMap<String, ConsumerBuckets>` and build
//! kernel buckets sized for each consumer's effective `RateLimit`.

use crate::error::{GatewayError, GatewayResult};
use cave_kernel::ratelimiter::{TokenBucket, TokenBucketConfig};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimit {
    /// Max requests per minute
    pub requests_per_minute: u32,
    /// Max tokens per minute (prompt + completion)
    pub tokens_per_minute: u32,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self { requests_per_minute: 60, tokens_per_minute: 100_000 }
    }
}

/// Two kernel buckets per consumer: one for request count, one for
/// cost-weighted prompt+completion tokens.
#[derive(Clone)]
struct ConsumerBuckets {
    request_bucket: TokenBucket,
    token_bucket: TokenBucket,
}

impl ConsumerBuckets {
    fn from_limit(limit: &RateLimit) -> Self {
        let req_capacity = f64::from(limit.requests_per_minute).max(1.0);
        let tok_capacity = f64::from(limit.tokens_per_minute).max(1.0);
        Self {
            request_bucket: TokenBucket::new(TokenBucketConfig::new(
                req_capacity,
                req_capacity / 60.0,
            )),
            token_bucket: TokenBucket::new(TokenBucketConfig::new(
                tok_capacity,
                tok_capacity / 60.0,
            )),
        }
    }
}

pub struct RateLimiter {
    consumers: DashMap<String, ConsumerBuckets>,
    default_limit: RateLimit,
    custom_limits: DashMap<String, RateLimit>,
}

impl RateLimiter {
    pub fn new(default_limit: RateLimit) -> Self {
        Self {
            consumers: DashMap::new(),
            default_limit,
            custom_limits: DashMap::new(),
        }
    }

    pub fn set_limit(&self, consumer: &str, limit: RateLimit) {
        self.custom_limits.insert(consumer.to_string(), limit);
        self.consumers.remove(consumer); // reset buckets when limits change
    }

    pub fn get_limit(&self, consumer: &str) -> RateLimit {
        self.custom_limits
            .get(consumer)
            .map(|l| l.clone())
            .unwrap_or_else(|| self.default_limit.clone())
    }

    /// Check whether a request is allowed. Consumes 1 request + `token_cost` tokens.
    pub fn check(&self, consumer: &str, token_cost: u32) -> GatewayResult<()> {
        let limit = self.get_limit(consumer);
        let entry = self
            .consumers
            .entry(consumer.to_string())
            .or_insert_with(|| ConsumerBuckets::from_limit(&limit));

        entry
            .request_bucket
            .try_consume_or_retry(1.0)
            .map_err(|wait| GatewayError::RateLimitExceeded {
                consumer: consumer.to_string(),
                retry_after_ms: duration_to_ms_ceil(wait),
            })?;

        if token_cost > 0 {
            entry
                .token_bucket
                .try_consume_or_retry(f64::from(token_cost))
                .map_err(|wait| GatewayError::RateLimitExceeded {
                    consumer: consumer.to_string(),
                    retry_after_ms: duration_to_ms_ceil(wait),
                })?;
        }

        Ok(())
    }

    /// Reset all buckets for a consumer (e.g., after upgrading their plan).
    pub fn reset(&self, consumer: &str) {
        self.consumers.remove(consumer);
    }

    pub fn list_consumers(&self) -> Vec<String> {
        self.consumers.iter().map(|e| e.key().clone()).collect()
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(RateLimit::default())
    }
}

/// Convert a `Duration` from `try_consume_or_retry` into the
/// gateway's `retry_after_ms: u64` wire field. Sub-millisecond waits
/// round up to 1ms so clients never see a zero retry hint.
fn duration_to_ms_ceil(d: std::time::Duration) -> u64 {
    let nanos = d.as_nanos();
    let ms = nanos.div_ceil(1_000_000);
    u64::try_from(ms).unwrap_or(u64::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_rate_limiting() {
        let limiter = RateLimiter::new(RateLimit { requests_per_minute: 3, tokens_per_minute: 1000 });
        // Should allow 3 requests
        assert!(limiter.check("user-1", 10).is_ok());
        assert!(limiter.check("user-1", 10).is_ok());
        assert!(limiter.check("user-1", 10).is_ok());
        // 4th should fail
        assert!(limiter.check("user-1", 10).is_err());
    }

    #[test]
    fn different_consumers_are_independent() {
        let limiter = RateLimiter::new(RateLimit { requests_per_minute: 1, tokens_per_minute: 1000 });
        assert!(limiter.check("user-a", 0).is_ok());
        assert!(limiter.check("user-b", 0).is_ok()); // separate bucket
        assert!(limiter.check("user-a", 0).is_err()); // user-a exhausted
    }

    #[test]
    fn custom_limit_overrides_default() {
        let limiter = RateLimiter::new(RateLimit { requests_per_minute: 1, tokens_per_minute: 100 });
        limiter.set_limit("premium", RateLimit { requests_per_minute: 100, tokens_per_minute: 1_000_000 });
        for _ in 0..10 {
            assert!(limiter.check("premium", 0).is_ok());
        }
    }

    #[test]
    fn token_exhaustion() {
        let limiter = RateLimiter::new(RateLimit { requests_per_minute: 1000, tokens_per_minute: 50 });
        assert!(limiter.check("user", 40).is_ok());
        assert!(limiter.check("user", 40).is_err()); // 40 > remaining 10
    }

    /// Sweep-006 regression — exhausted consumer receives a non-zero
    /// `retry_after_ms` so the HTTP layer can set a Retry-After header.
    #[test]
    fn rejection_carries_nonzero_retry_after() {
        let limiter = RateLimiter::new(RateLimit { requests_per_minute: 1, tokens_per_minute: 100 });
        // burn the single request slot
        assert!(limiter.check("user", 1).is_ok());
        let err = limiter.check("user", 1).expect_err("second call exhausts request_bucket");
        match err {
            GatewayError::RateLimitExceeded { retry_after_ms, .. } => {
                assert!(retry_after_ms > 0, "Retry-After must be > 0, got {retry_after_ms}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
