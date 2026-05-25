// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Quota subsystem — OpenBao `vault/quotas/`.
//!
//! Two quota kinds match upstream:
//!   * **rate-limit** — Token-bucket request rate limiter scoped to a
//!     path prefix, mount accessor or namespace. Mirrors
//!     `vault/quotas/quotas.go::Manager` + `quotas/quota_rate_limit.go`.
//!   * **lease-count** — Hard upper bound on the number of concurrent
//!     leases a tenant may hold. Mirrors `quotas/quota_lease_count.go`.
//!
//! Both kinds share the [`QuotaRule`] header (id, name, path, role
//! filter, inheritable flag) and live in a single [`QuotaStore`] keyed
//! by quota id.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Common header shared by rate-limit + lease-count quotas. Maps to
/// `quotas/quota.go::Quota` (interface) — the fields below are the
/// concrete struct the JSON API exposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaRule {
    pub id: String,
    pub name: String,
    /// Path prefix the quota applies to, e.g. `"kv/"` or `""` for global.
    pub path: String,
    /// Mount accessor filter. Empty == all mounts.
    pub mount: String,
    /// Namespace filter. Empty == root namespace.
    pub namespace_id: String,
    /// If `true` child namespaces inherit the same rule.
    pub inheritable: bool,
}

/// A rate-limit quota — token-bucket with `rate` tokens per second and
/// a burst of `rate * interval_seconds`. Mirrors
/// `quotas/quota_rate_limit.go::RateLimitQuota`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitQuota {
    pub rule: QuotaRule,
    /// Sustained tokens-per-second.
    pub rate: f64,
    /// Refill window in seconds; the bucket size is `rate * interval`.
    pub interval_seconds: u32,
    /// If `true`, OpenBao replies 429 once exhausted; if `false`, it
    /// blocks briefly and retries (upstream's "block" mode).
    pub block_on_exhaustion: bool,
}

/// A lease-count quota — hard cap on concurrent leases. Mirrors
/// `quotas/quota_lease_count.go::LeaseCountQuota`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseCountQuota {
    pub rule: QuotaRule,
    pub max_leases: u64,
}

/// Mutable token-bucket state — owns `tokens` and `last_refill_epoch`.
#[derive(Debug, Clone)]
struct Bucket {
    tokens: f64,
    last_refill_epoch: i64,
    capacity: f64,
    rate: f64,
}

impl Bucket {
    fn refill(&mut self, now: i64) {
        let elapsed = (now - self.last_refill_epoch).max(0) as f64;
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
            self.last_refill_epoch = now;
        }
    }

    fn try_take(&mut self, now: i64) -> bool {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Quota store — the OpenBao `quotas.Manager` analog. Thread-safe via
/// an internal `Mutex` so the runtime can share an `Arc<QuotaStore>`
/// across handlers without `RwLock` contention on the hot path.
pub struct QuotaStore {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    rate_limits: HashMap<String, RateLimitQuota>,
    lease_counts: HashMap<String, LeaseCountQuota>,
    buckets: HashMap<String, Bucket>,
    lease_counters: HashMap<String, u64>,
}

impl Default for QuotaStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }
}

/// Decision returned by [`QuotaStore::check`]. Mirrors the upstream
/// `ResponseStatus` flavour: pass, throttle (429), or block-then-fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaDecision {
    Allow,
    RateLimited { quota_id: String, retry_in_ms: u32 },
    LeaseLimited { quota_id: String, max_leases: u64 },
}

impl QuotaStore {
    /// Register or replace a rate-limit quota. Mirrors `Manager.SetQuota`.
    pub fn put_rate_limit(&self, q: RateLimitQuota) {
        let mut i = self.inner.lock().unwrap();
        let bucket = Bucket {
            tokens: q.rate * q.interval_seconds as f64,
            last_refill_epoch: 0,
            capacity: q.rate * q.interval_seconds as f64,
            rate: q.rate,
        };
        i.buckets.insert(q.rule.id.clone(), bucket);
        i.rate_limits.insert(q.rule.id.clone(), q);
    }

    /// Register or replace a lease-count quota.
    pub fn put_lease_count(&self, q: LeaseCountQuota) {
        let mut i = self.inner.lock().unwrap();
        i.lease_counters.entry(q.rule.id.clone()).or_insert(0);
        i.lease_counts.insert(q.rule.id.clone(), q);
    }

    /// Delete a quota by id. Returns whether something was removed.
    pub fn delete(&self, id: &str) -> bool {
        let mut i = self.inner.lock().unwrap();
        i.buckets.remove(id);
        i.lease_counters.remove(id);
        i.rate_limits.remove(id).is_some() || i.lease_counts.remove(id).is_some()
    }

    /// List all rate-limit quotas sorted by id.
    pub fn list_rate_limits(&self) -> Vec<RateLimitQuota> {
        let i = self.inner.lock().unwrap();
        let mut out: Vec<_> = i.rate_limits.values().cloned().collect();
        out.sort_by(|a, b| a.rule.id.cmp(&b.rule.id));
        out
    }

    /// List all lease-count quotas sorted by id.
    pub fn list_lease_counts(&self) -> Vec<LeaseCountQuota> {
        let i = self.inner.lock().unwrap();
        let mut out: Vec<_> = i.lease_counts.values().cloned().collect();
        out.sort_by(|a, b| a.rule.id.cmp(&b.rule.id));
        out
    }

    /// Evaluate the most-specific applicable quota for a request to
    /// `(path, namespace_id)`. Returns the first denial; falls through
    /// to `Allow` when no quota matches.
    pub fn check(&self, path: &str, namespace_id: &str, now_epoch: i64) -> QuotaDecision {
        let mut i = self.inner.lock().unwrap();

        let rate_ids: Vec<String> = i
            .rate_limits
            .values()
            .filter(|q| Self::rule_applies(&q.rule, path, namespace_id))
            .map(|q| q.rule.id.clone())
            .collect();
        for id in rate_ids {
            let allowed = i
                .buckets
                .get_mut(&id)
                .map(|b| b.try_take(now_epoch))
                .unwrap_or(true);
            if !allowed {
                let q = i.rate_limits.get(&id).unwrap();
                let retry_in_ms = ((1.0 / q.rate.max(0.0001)) * 1000.0) as u32;
                return QuotaDecision::RateLimited {
                    quota_id: id,
                    retry_in_ms,
                };
            }
        }

        let lease_hits: Vec<(String, u64, u64)> = i
            .lease_counts
            .values()
            .filter(|q| Self::rule_applies(&q.rule, path, namespace_id))
            .map(|q| {
                let used = *i.lease_counters.get(&q.rule.id).unwrap_or(&0);
                (q.rule.id.clone(), used, q.max_leases)
            })
            .collect();
        for (id, used, max) in lease_hits {
            if used >= max {
                return QuotaDecision::LeaseLimited {
                    quota_id: id,
                    max_leases: max,
                };
            }
        }

        QuotaDecision::Allow
    }

    /// Account for a lease grant — increment the matching counters by 1.
    pub fn record_lease_grant(&self, path: &str, namespace_id: &str) {
        let mut i = self.inner.lock().unwrap();
        let ids: Vec<String> = i
            .lease_counts
            .values()
            .filter(|q| Self::rule_applies(&q.rule, path, namespace_id))
            .map(|q| q.rule.id.clone())
            .collect();
        for id in ids {
            *i.lease_counters.entry(id).or_insert(0) += 1;
        }
    }

    /// Account for a lease release — decrement, never below zero.
    pub fn record_lease_release(&self, path: &str, namespace_id: &str) {
        let mut i = self.inner.lock().unwrap();
        let ids: Vec<String> = i
            .lease_counts
            .values()
            .filter(|q| Self::rule_applies(&q.rule, path, namespace_id))
            .map(|q| q.rule.id.clone())
            .collect();
        for id in ids {
            let counter = i.lease_counters.entry(id).or_insert(0);
            if *counter > 0 {
                *counter -= 1;
            }
        }
    }

    fn rule_applies(rule: &QuotaRule, path: &str, namespace_id: &str) -> bool {
        if !rule.namespace_id.is_empty() && rule.namespace_id != namespace_id {
            return false;
        }
        if !rule.path.is_empty() && !path.starts_with(&rule.path) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, path: &str) -> QuotaRule {
        QuotaRule {
            id: id.into(),
            name: id.into(),
            path: path.into(),
            mount: String::new(),
            namespace_id: String::new(),
            inheritable: false,
        }
    }

    #[test]
    fn rate_limit_quota_admits_within_capacity() {
        let s = QuotaStore::default();
        s.put_rate_limit(RateLimitQuota {
            rule: rule("rl-1", "kv/"),
            rate: 2.0,
            interval_seconds: 1,
            block_on_exhaustion: false,
        });
        // Bucket capacity = 2 tokens. Two consecutive requests pass.
        assert_eq!(s.check("kv/data/foo", "", 100), QuotaDecision::Allow);
        assert_eq!(s.check("kv/data/foo", "", 100), QuotaDecision::Allow);
    }

    #[test]
    fn rate_limit_quota_denies_after_capacity_exhausted() {
        let s = QuotaStore::default();
        s.put_rate_limit(RateLimitQuota {
            rule: rule("rl-1", "kv/"),
            rate: 1.0,
            interval_seconds: 1,
            block_on_exhaustion: false,
        });
        // Capacity = 1.
        assert_eq!(s.check("kv/foo", "", 100), QuotaDecision::Allow);
        let decision = s.check("kv/foo", "", 100);
        assert!(matches!(decision, QuotaDecision::RateLimited { .. }));
    }

    #[test]
    fn rate_limit_quota_refills_after_interval() {
        let s = QuotaStore::default();
        s.put_rate_limit(RateLimitQuota {
            rule: rule("rl-1", ""),
            rate: 1.0,
            interval_seconds: 1,
            block_on_exhaustion: false,
        });
        assert_eq!(s.check("any", "", 100), QuotaDecision::Allow);
        // Exhausted at t=100
        assert!(matches!(
            s.check("any", "", 100),
            QuotaDecision::RateLimited { .. }
        ));
        // 5 s later → 5 tokens added → allowed again
        assert_eq!(s.check("any", "", 105), QuotaDecision::Allow);
    }

    #[test]
    fn lease_count_quota_blocks_at_cap() {
        let s = QuotaStore::default();
        s.put_lease_count(LeaseCountQuota {
            rule: rule("lc-1", ""),
            max_leases: 2,
        });
        s.record_lease_grant("any", "");
        s.record_lease_grant("any", "");
        // Cap is 2, two grants outstanding → next check denies.
        let decision = s.check("any", "", 100);
        assert!(matches!(decision, QuotaDecision::LeaseLimited { .. }));
    }

    #[test]
    fn lease_count_quota_releases_decrement_counter() {
        let s = QuotaStore::default();
        s.put_lease_count(LeaseCountQuota {
            rule: rule("lc-1", ""),
            max_leases: 1,
        });
        s.record_lease_grant("any", "");
        assert!(matches!(
            s.check("any", "", 100),
            QuotaDecision::LeaseLimited { .. }
        ));
        s.record_lease_release("any", "");
        assert_eq!(s.check("any", "", 100), QuotaDecision::Allow);
    }

    #[test]
    fn lease_count_quota_release_saturates_at_zero() {
        let s = QuotaStore::default();
        s.put_lease_count(LeaseCountQuota {
            rule: rule("lc-1", ""),
            max_leases: 5,
        });
        // Many releases without grants must not underflow.
        for _ in 0..10 {
            s.record_lease_release("any", "");
        }
        let i = s.inner.lock().unwrap();
        assert_eq!(*i.lease_counters.get("lc-1").unwrap(), 0);
    }

    #[test]
    fn rate_limit_scoped_to_path_prefix() {
        let s = QuotaStore::default();
        s.put_rate_limit(RateLimitQuota {
            rule: rule("rl-kv", "kv/"),
            rate: 1.0,
            interval_seconds: 1,
            block_on_exhaustion: false,
        });
        assert_eq!(s.check("kv/foo", "", 100), QuotaDecision::Allow);
        // First request consumed the kv bucket; pki path is untouched.
        assert_eq!(s.check("pki/issue/role", "", 100), QuotaDecision::Allow);
    }

    #[test]
    fn rate_limit_scoped_to_namespace() {
        let s = QuotaStore::default();
        let mut r = rule("rl-ns", "");
        r.namespace_id = "tenant-a".into();
        s.put_rate_limit(RateLimitQuota {
            rule: r,
            rate: 1.0,
            interval_seconds: 1,
            block_on_exhaustion: false,
        });
        assert_eq!(s.check("any", "tenant-a", 100), QuotaDecision::Allow);
        // Different tenant — quota does not apply, allow.
        assert_eq!(s.check("any", "tenant-b", 100), QuotaDecision::Allow);
        assert_eq!(s.check("any", "tenant-b", 100), QuotaDecision::Allow);
    }

    #[test]
    fn quota_crud_round_trip() {
        let s = QuotaStore::default();
        s.put_rate_limit(RateLimitQuota {
            rule: rule("rl-1", ""),
            rate: 5.0,
            interval_seconds: 1,
            block_on_exhaustion: false,
        });
        s.put_lease_count(LeaseCountQuota {
            rule: rule("lc-1", ""),
            max_leases: 10,
        });
        assert_eq!(s.list_rate_limits().len(), 1);
        assert_eq!(s.list_lease_counts().len(), 1);
        assert!(s.delete("rl-1"));
        assert!(s.delete("lc-1"));
        assert!(!s.delete("nope"));
        assert_eq!(s.list_rate_limits().len(), 0);
        assert_eq!(s.list_lease_counts().len(), 0);
    }

    #[test]
    fn rate_limit_quota_retry_in_ms_inversely_proportional_to_rate() {
        let s = QuotaStore::default();
        s.put_rate_limit(RateLimitQuota {
            rule: rule("rl-1", ""),
            rate: 1.0,
            interval_seconds: 1,
            block_on_exhaustion: false,
        });
        assert_eq!(s.check("any", "", 100), QuotaDecision::Allow);
        let d = s.check("any", "", 100);
        match d {
            QuotaDecision::RateLimited { retry_in_ms, .. } => {
                assert_eq!(retry_in_ms, 1000);
            }
            _ => panic!("expected rate limit"),
        }
    }

    #[test]
    fn quota_store_default_is_empty() {
        let s = QuotaStore::default();
        assert_eq!(s.list_rate_limits().len(), 0);
        assert_eq!(s.list_lease_counts().len(), 0);
        // No quotas at all → check always allows.
        assert_eq!(s.check("kv/foo", "tenant-1", 100), QuotaDecision::Allow);
    }
}
