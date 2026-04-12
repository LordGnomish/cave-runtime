//! Per-consumer rate limiting — token bucket per API key / user.

use crate::error::{GatewayError, GatewayResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

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

struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_rate: f64, // tokens per second
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, per_minute: f64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_rate: per_minute / 60.0,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;
    }

    /// Returns Ok if tokens are available, Err with retry_after_ms on exhaustion.
    fn try_consume(&mut self, amount: f64) -> Result<(), u64> {
        self.refill();
        if self.tokens >= amount {
            self.tokens -= amount;
            Ok(())
        } else {
            let deficit = amount - self.tokens;
            let wait_secs = deficit / self.refill_rate;
            Err((wait_secs * 1000.0) as u64 + 1)
        }
    }
}

struct ConsumerBuckets {
    request_bucket: TokenBucket,
    token_bucket: TokenBucket,
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
        self.custom_limits.get(consumer).map(|l| l.clone()).unwrap_or_else(|| self.default_limit.clone())
    }

    /// Check whether a request is allowed. Consumes 1 request + `token_cost` tokens.
    pub fn check(&self, consumer: &str, token_cost: u32) -> GatewayResult<()> {
        let limit = self.get_limit(consumer);

        let mut entry = self.consumers.entry(consumer.to_string()).or_insert_with(|| ConsumerBuckets {
            request_bucket: TokenBucket::new(limit.requests_per_minute as f64, limit.requests_per_minute as f64),
            token_bucket: TokenBucket::new(limit.tokens_per_minute as f64, limit.tokens_per_minute as f64),
        });

        entry.request_bucket.try_consume(1.0).map_err(|retry_ms| {
            GatewayError::RateLimitExceeded { consumer: consumer.to_string(), retry_after_ms: retry_ms }
        })?;

        if token_cost > 0 {
            entry.token_bucket.try_consume(token_cost as f64).map_err(|retry_ms| {
                GatewayError::RateLimitExceeded { consumer: consumer.to_string(), retry_after_ms: retry_ms }
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
}
