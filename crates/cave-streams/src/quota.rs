// SPDX-License-Identifier: AGPL-3.0-or-later
//! Kafka quota management — produce/fetch byte rate and request rate limits.

use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

// ── Quota definition ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quota {
    /// Producer byte rate limit (bytes/sec), None = unlimited
    pub producer_byte_rate: Option<f64>,
    /// Consumer byte rate limit (bytes/sec), None = unlimited
    pub consumer_byte_rate: Option<f64>,
    /// Request rate as a fraction of total resources (0.0–1.0), None = unlimited
    pub request_percentage: Option<f64>,
    /// Controller mutation rate (ops/sec), None = unlimited
    pub controller_mutation_rate: Option<f64>,
}

impl Quota {
    pub fn unlimited() -> Self {
        Self {
            producer_byte_rate: None,
            consumer_byte_rate: None,
            request_percentage: None,
            controller_mutation_rate: None,
        }
    }

    pub fn producer(bytes_per_sec: f64) -> Self {
        Self {
            producer_byte_rate: Some(bytes_per_sec),
            ..Self::unlimited()
        }
    }

    pub fn consumer(bytes_per_sec: f64) -> Self {
        Self {
            consumer_byte_rate: Some(bytes_per_sec),
            ..Self::unlimited()
        }
    }
}

// ── Entity type ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    User,
    ClientId,
    Ip,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QuotaEntity {
    pub entity_type: EntityType,
    pub entity_name: Option<String>, // None = default quota
}

impl QuotaEntity {
    pub fn user(name: &str) -> Self {
        Self {
            entity_type: EntityType::User,
            entity_name: Some(name.to_string()),
        }
    }

    pub fn client(id: &str) -> Self {
        Self {
            entity_type: EntityType::ClientId,
            entity_name: Some(id.to_string()),
        }
    }

    pub fn default_user() -> Self {
        Self {
            entity_type: EntityType::User,
            entity_name: None,
        }
    }
}

// ── Token bucket rate limiter ─────────────────────────────────────────────────

#[derive(Debug)]
struct TokenBucket {
    /// Current tokens available
    tokens: f64,
    /// Maximum tokens (burst)
    capacity: f64,
    /// Refill rate (tokens/sec)
    refill_rate: f64,
    /// Last refill timestamp (ms)
    last_refill_ms: i64,
}

impl TokenBucket {
    fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_rate,
            last_refill_ms: Utc::now().timestamp_millis(),
        }
    }

    #[allow(dead_code)]
    fn try_consume(&mut self, amount: f64) -> bool {
        self.refill();
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Utc::now().timestamp_millis();
        let elapsed_sec = (now - self.last_refill_ms) as f64 / 1000.0;
        self.tokens = (self.tokens + elapsed_sec * self.refill_rate).min(self.capacity);
        self.last_refill_ms = now;
    }

    /// Throttle time in ms (how long until `amount` tokens will be available).
    fn throttle_ms(&mut self, amount: f64) -> i32 {
        self.refill();
        if self.tokens >= amount {
            return 0;
        }
        let deficit = amount - self.tokens;
        ((deficit / self.refill_rate) * 1000.0) as i32
    }
}

// ── Quota manager ─────────────────────────────────────────────────────────────

pub struct QuotaManager {
    /// entity → quota definition
    quotas: DashMap<QuotaEntity, Quota>,
    /// entity → token buckets for enforcement
    producer_buckets: DashMap<String, TokenBucket>,
    consumer_buckets: DashMap<String, TokenBucket>,
    request_buckets: DashMap<String, TokenBucket>,
}

impl QuotaManager {
    pub fn new() -> Self {
        Self {
            quotas: DashMap::new(),
            producer_buckets: DashMap::new(),
            consumer_buckets: DashMap::new(),
            request_buckets: DashMap::new(),
        }
    }

    pub fn set_quota(&self, entity: QuotaEntity, quota: Quota) {
        self.quotas.insert(entity, quota);
    }

    pub fn get_quota(&self, entity: &QuotaEntity) -> Option<Quota> {
        self.quotas.get(entity).map(|q| q.clone())
    }

    pub fn delete_quota(&self, entity: &QuotaEntity) -> bool {
        self.quotas.remove(entity).is_some()
    }

    pub fn list_quotas(&self) -> Vec<(QuotaEntity, Quota)> {
        self.quotas
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    /// Check produce quota. Returns throttle_ms (0 = not throttled).
    pub fn check_produce_quota(&self, principal: &str, bytes: f64) -> i32 {
        let Some(quota) = self.find_quota_for(principal) else {
            return 0;
        };
        let Some(rate) = quota.producer_byte_rate else {
            return 0;
        };
        let mut bucket = self
            .producer_buckets
            .entry(principal.to_string())
            .or_insert_with(|| TokenBucket::new(rate, rate));
        bucket.throttle_ms(bytes)
    }

    /// Check fetch quota. Returns throttle_ms (0 = not throttled).
    pub fn check_fetch_quota(&self, principal: &str, bytes: f64) -> i32 {
        let Some(quota) = self.find_quota_for(principal) else {
            return 0;
        };
        let Some(rate) = quota.consumer_byte_rate else {
            return 0;
        };
        let mut bucket = self
            .consumer_buckets
            .entry(principal.to_string())
            .or_insert_with(|| TokenBucket::new(rate, rate));
        bucket.throttle_ms(bytes)
    }

    /// Check request rate quota. Returns throttle_ms.
    pub fn check_request_quota(&self, principal: &str, request_rate_fraction: f64) -> i32 {
        let Some(quota) = self.find_quota_for(principal) else {
            return 0;
        };
        let Some(limit) = quota.request_percentage else {
            return 0;
        };
        let capacity = limit * 1000.0; // treat % as tokens per second scaled
        let mut bucket = self
            .request_buckets
            .entry(principal.to_string())
            .or_insert_with(|| TokenBucket::new(capacity, capacity));
        bucket.throttle_ms(request_rate_fraction * 1000.0)
    }

    fn find_quota_for(&self, principal: &str) -> Option<Quota> {
        // Try exact user match first, then default
        let user_entity = QuotaEntity::user(principal);
        if let Some(q) = self.quotas.get(&user_entity) {
            return Some(q.clone());
        }
        let default_entity = QuotaEntity::default_user();
        self.quotas.get(&default_entity).map(|q| q.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get_quota() {
        let qm = QuotaManager::new();
        qm.set_quota(QuotaEntity::user("alice"), Quota::producer(1_048_576.0));
        let q = qm.get_quota(&QuotaEntity::user("alice")).unwrap();
        assert_eq!(q.producer_byte_rate, Some(1_048_576.0));
    }

    #[test]
    fn delete_quota() {
        let qm = QuotaManager::new();
        qm.set_quota(QuotaEntity::user("bob"), Quota::unlimited());
        assert!(qm.delete_quota(&QuotaEntity::user("bob")));
        assert!(qm.get_quota(&QuotaEntity::user("bob")).is_none());
    }

    #[test]
    fn no_quota_means_not_throttled() {
        let qm = QuotaManager::new();
        assert_eq!(qm.check_produce_quota("nobody", 1_000_000.0), 0);
    }

    #[test]
    fn quota_throttles_large_request() {
        let qm = QuotaManager::new();
        // 1 byte/sec quota
        qm.set_quota(QuotaEntity::user("limited"), Quota::producer(1.0));
        // Requesting 1 MB should require throttling
        let throttle = qm.check_produce_quota("limited", 1_048_576.0);
        assert!(throttle > 0, "expected throttle > 0, got {throttle}");
    }
}
