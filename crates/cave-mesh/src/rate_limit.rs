//! Token-bucket rate limiter per service.
//!
//! Each service gets an independent bucket whose refill rate is derived from
//! the configured `RateLimitPolicy` (requests per second/minute/hour).

use crate::models::{RateLimitPolicy, RateLimitUnit};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};
use tracing::debug;

// ─────────────────────────────────────────────────────────────
// Token Bucket
// ─────────────────────────────────────────────────────────────

struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_rate: f64, // tokens per second
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            capacity,
            tokens: capacity, // start full
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Returns `true` if a token was consumed (request allowed).
    fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = Instant::now();
    }

    fn available_tokens(&self) -> f64 {
        self.tokens
    }
}

// ─────────────────────────────────────────────────────────────
// RateLimiter
// ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum RateLimitDecision {
    Allowed,
    Denied { service: String },
}

/// Thread-safe rate limiter keyed by service name.
#[derive(Clone)]
pub struct RateLimiter {
    policies: Arc<RwLock<HashMap<String, RateLimitPolicy>>>,
    buckets: Arc<Mutex<HashMap<String, TokenBucket>>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // ─── Policy CRUD ─────────────────────────────────────────

    pub fn upsert_policy(&self, policy: RateLimitPolicy) {
        let name = policy.name.clone();
        {
            let mut map = self.policies.write().unwrap();
            map.insert(name.clone(), policy.clone());
        }
        // Rebuild bucket with new limits
        if let Some(rule) = policy.rules.first() {
            let rps = rule.unit.to_rps(rule.requests_per_unit);
            // capacity = burst: allow up to 2× the per-second rate
            let capacity = (rps * 2.0).max(1.0);
            let mut buckets = self.buckets.lock().unwrap();
            buckets.insert(name, TokenBucket::new(capacity, rps));
        }
    }

    pub fn remove_policy(&self, name: &str) {
        self.policies.write().unwrap().remove(name);
        self.buckets.lock().unwrap().remove(name);
    }

    pub fn list_policies(&self) -> Vec<RateLimitPolicy> {
        self.policies.read().unwrap().values().cloned().collect()
    }

    // ─── Enforcement ─────────────────────────────────────────

    /// Try to consume a token for `service`.  Returns `Allowed` if a matching
    /// policy exists AND a token is available, or if no policy is configured.
    pub fn check_and_consume(&self, service: &str) -> RateLimitDecision {
        let has_policy = {
            let map = self.policies.read().unwrap();
            map.contains_key(service)
        };

        if !has_policy {
            return RateLimitDecision::Allowed;
        }

        let mut buckets = self.buckets.lock().unwrap();

        // Ensure bucket exists (in case it was removed or never created)
        let bucket = buckets.entry(service.to_string()).or_insert_with(|| {
            let rps = self.default_rps_for(service);
            TokenBucket::new((rps * 2.0).max(1.0), rps)
        });

        if bucket.try_consume() {
            debug!(service = %service, tokens = %bucket.available_tokens(), "Rate limit: allowed");
            RateLimitDecision::Allowed
        } else {
            debug!(service = %service, "Rate limit: denied");
            RateLimitDecision::Denied {
                service: service.to_string(),
            }
        }
    }

    fn default_rps_for(&self, service: &str) -> f64 {
        let map = self.policies.read().unwrap();
        map.get(service)
            .and_then(|p| p.rules.first())
            .map(|r| r.unit.to_rps(r.requests_per_unit))
            .unwrap_or(100.0)
    }

    /// Snapshot of all bucket states.
    pub fn snapshot(&self) -> Vec<RateLimiterSnapshot> {
        let buckets = self.buckets.lock().unwrap();
        buckets
            .iter()
            .map(|(k, b)| RateLimiterSnapshot {
                service: k.clone(),
                available_tokens: b.available_tokens(),
                capacity: b.capacity,
                refill_rate_rps: b.refill_rate,
            })
            .collect()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RateLimiterSnapshot {
    pub service: String,
    pub available_tokens: f64,
    pub capacity: f64,
    pub refill_rate_rps: f64,
}

// ─────────────────────────────────────────────────────────────
// Direct constructor for tests
// ─────────────────────────────────────────────────────────────

impl RateLimiter {
    /// Create a rate-limited policy directly (helper for tests).
    pub fn with_policy(
        service: impl Into<String>,
        requests_per_second: u64,
    ) -> Self {
        let rl = Self::new();
        let name = service.into();
        rl.upsert_policy(RateLimitPolicy {
            name: name.clone(),
            namespace: "default".to_string(),
            selector: None,
            rules: vec![crate::models::RateLimitRule {
                requests_per_unit: requests_per_second,
                unit: RateLimitUnit::Second,
            }],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        });
        rl
    }
}
