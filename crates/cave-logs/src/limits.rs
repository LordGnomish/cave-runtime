// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-tenant rate limiting and query limits.
//!
//! Uses a token-bucket algorithm for ingestion rate limiting:
//!   - Each tenant starts with `burst` tokens.
//!   - Tokens refill at `rate` bytes/second.
//!   - An ingest request consuming `n` bytes is rejected if < n tokens available.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use parking_lot::Mutex;
use chrono::Utc;

use crate::models::{TenantId, TenantLimits};

/// Token bucket state for one tenant.
struct Bucket {
    /// Current available tokens (bytes).
    tokens: f64,
    /// Maximum tokens (burst size).
    capacity: f64,
    /// Refill rate in tokens/second.
    rate: f64,
    /// Last time tokens were refilled.
    last_refill: Instant,
}

impl Bucket {
    fn new(rate_bytes_per_sec: u64, burst_bytes: u64) -> Self {
        Self {
            tokens: burst_bytes as f64,
            capacity: burst_bytes as f64,
            rate: rate_bytes_per_sec as f64,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume `n` tokens. Returns `true` if allowed.
    fn try_consume(&mut self, n: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
        self.last_refill = now;

        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }
}

/// Global limits registry — one set of limits per tenant.
pub struct LimitsRegistry {
    defaults: TenantLimits,
    overrides: HashMap<TenantId, TenantLimits>,
    buckets: Mutex<HashMap<TenantId, Bucket>>,
}

impl LimitsRegistry {
    pub fn new(defaults: TenantLimits) -> Arc<Self> {
        Arc::new(Self {
            defaults,
            overrides: HashMap::new(),
            buckets: Mutex::new(HashMap::new()),
        })
    }

    pub fn with_defaults() -> Arc<Self> {
        Self::new(TenantLimits::default())
    }

    /// Set per-tenant limit overrides.
    pub fn set_tenant_limits(&mut self, tenant: &str, limits: TenantLimits) {
        self.overrides.insert(tenant.to_owned(), limits);
    }

    /// Get effective limits for a tenant.
    pub fn limits_for(&self, tenant: &str) -> &TenantLimits {
        self.overrides.get(tenant).unwrap_or(&self.defaults)
    }

    /// Check and consume ingestion quota for `byte_count` bytes.
    /// Returns `Ok(())` if allowed, `Err` with message if rate-limited.
    pub fn check_ingestion_rate(&self, tenant: &str, byte_count: usize) -> Result<(), LimitError> {
        let limits = self.limits_for(tenant);
        if limits.ingestion_rate_bytes == 0 {
            return Ok(()); // unlimited
        }

        let mut buckets = self.buckets.lock();
        let bucket = buckets
            .entry(tenant.to_owned())
            .or_insert_with(|| Bucket::new(limits.ingestion_rate_bytes, limits.ingestion_burst_bytes));

        if bucket.try_consume(byte_count as f64) {
            Ok(())
        } else {
            Err(LimitError::RateLimited {
                tenant: tenant.to_owned(),
                limit_bytes_per_sec: limits.ingestion_rate_bytes,
            })
        }
    }

    /// Validate a log line against size limits.
    pub fn check_line_size(&self, tenant: &str, line_len: usize) -> Result<(), LimitError> {
        let limits = self.limits_for(tenant);
        if limits.max_line_size > 0 && line_len > limits.max_line_size {
            return Err(LimitError::LineTooLong {
                tenant: tenant.to_owned(),
                len: line_len,
                max: limits.max_line_size,
            });
        }
        Ok(())
    }

    /// Validate query limits: result count and time range.
    pub fn check_query_limits(
        &self,
        tenant: &str,
        limit: usize,
        start_ns: i64,
        end_ns: i64,
    ) -> Result<usize, LimitError> {
        let limits = self.limits_for(tenant);
        let effective_limit = limit.min(limits.max_entries_per_query);

        if limits.max_query_range_hours > 0 {
            let range_ns = end_ns - start_ns;
            let max_range_ns = limits.max_query_range_hours as i64 * 3_600_000_000_000;
            if range_ns > max_range_ns {
                return Err(LimitError::QueryRangeTooLong {
                    tenant: tenant.to_owned(),
                    range_hours: (range_ns / 3_600_000_000_000) as u64,
                    max_hours: limits.max_query_range_hours,
                });
            }
        }

        Ok(effective_limit)
    }

    /// Check whether a tenant has exceeded stream count.
    pub fn check_stream_count(&self, tenant: &str, current_count: u64) -> Result<(), LimitError> {
        let limits = self.limits_for(tenant);
        if limits.max_streams > 0 && current_count >= limits.max_streams {
            return Err(LimitError::TooManyStreams {
                tenant: tenant.to_owned(),
                count: current_count,
                max: limits.max_streams,
            });
        }
        Ok(())
    }

    /// Retention cutoff timestamp (nanoseconds) for a tenant.
    pub fn retention_cutoff_ns(&self, tenant: &str) -> i64 {
        let limits = self.limits_for(tenant);
        let retention_secs = if limits.retention_hours > 0 {
            limits.retention_hours as i64 * 3600
        } else {
            7 * 24 * 3600 // default 7 days
        };
        (Utc::now().timestamp() - retention_secs) * 1_000_000_000
    }
}

/// A limit violation error.
#[derive(Debug, thiserror::Error)]
pub enum LimitError {
    #[error("ingestion rate limit exceeded for tenant {tenant}: limit is {limit_bytes_per_sec} bytes/s")]
    RateLimited { tenant: String, limit_bytes_per_sec: u64 },

    #[error("log line too long for tenant {tenant}: {len} bytes > {max} bytes limit")]
    LineTooLong { tenant: String, len: usize, max: usize },

    #[error("query range too long for tenant {tenant}: {range_hours}h > {max_hours}h limit")]
    QueryRangeTooLong { tenant: String, range_hours: u64, max_hours: u64 },

    #[error("too many streams for tenant {tenant}: {count} >= {max} limit")]
    TooManyStreams { tenant: String, count: u64, max: u64 },
}

impl LimitError {
    pub fn http_status(&self) -> u16 {
        match self {
            Self::RateLimited { .. } => 429,
            _ => 400,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_allows_within_burst() {
        let reg = LimitsRegistry::with_defaults();
        // Default burst is 16 MiB — a 1 KiB ingest should be fine.
        assert!(reg.check_ingestion_rate("tenant", 1024).is_ok());
    }

    #[test]
    fn rate_limit_denies_over_burst() {
        let mut limits = TenantLimits::default();
        limits.ingestion_burst_bytes = 100;
        limits.ingestion_rate_bytes = 100;
        let reg = LimitsRegistry::new(limits);
        // First 100 bytes fits in burst.
        assert!(reg.check_ingestion_rate("t", 100).is_ok());
        // Next byte should be denied (no time to refill).
        assert!(reg.check_ingestion_rate("t", 1).is_err());
    }

    #[test]
    fn line_size_ok() {
        let reg = LimitsRegistry::with_defaults();
        assert!(reg.check_line_size("t", 1000).is_ok());
    }

    #[test]
    fn line_size_rejected() {
        let mut limits = TenantLimits::default();
        limits.max_line_size = 100;
        let reg = LimitsRegistry::new(limits);
        assert!(reg.check_line_size("t", 101).is_err());
    }

    #[test]
    fn query_limit_clamped() {
        let reg = LimitsRegistry::with_defaults();
        // Requesting more than the max should be clamped.
        let effective = reg.check_query_limits("t", 1_000_000, 0, 3_600_000_000_000).unwrap();
        assert!(effective <= TenantLimits::default().max_entries_per_query);
    }

    #[test]
    fn query_range_rejected() {
        let mut limits = TenantLimits::default();
        limits.max_query_range_hours = 1;
        let reg = LimitsRegistry::new(limits);
        // 2h range should fail.
        let two_hours_ns = 2 * 3_600_000_000_000i64;
        assert!(reg.check_query_limits("t", 100, 0, two_hours_ns).is_err());
    }
}
