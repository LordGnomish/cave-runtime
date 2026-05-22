// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-tenant storage quota.
//!
//! Mirrors etcd v3.6.10 `server/storage/quota/`. Upstream `Quota` is
//! used as the gate for `Put`/`Txn` writes: every write asks the quota
//! "may I add N bytes and one key?" and, on refusal, the server raises
//! the `NO SPACE` alarm published via `maintenance.rs`.
//!
//! cave-etcd's MVP previously surfaced a single `db-size-bytes` alarm
//! through `maintenance.rs`; this module adds per-tenant accounting
//! keyed by key prefix, so multi-tenant deployments can fence one
//! noisy neighbour without raising the cluster-wide alarm.

use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageSnapshot {
    pub keys: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaLimits {
    pub max_keys: u64,
    pub max_bytes: u64,
}

impl QuotaLimits {
    pub const fn new(max_keys: u64, max_bytes: u64) -> Self {
        Self {
            max_keys,
            max_bytes,
        }
    }

    /// "Unbounded" sentinel — mirrors etcd's `0 = no limit`.
    pub const fn unbounded() -> Self {
        Self {
            max_keys: u64::MAX,
            max_bytes: u64::MAX,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaDecision {
    Allow,
    DenyMaxKeys { current: u64, limit: u64 },
    DenyMaxBytes { current: u64, limit: u64 },
}

impl QuotaDecision {
    pub fn is_allow(&self) -> bool {
        matches!(self, QuotaDecision::Allow)
    }
}

#[derive(Debug, Default)]
struct TenantState {
    usage: UsageSnapshot,
    limits: Option<QuotaLimits>,
}

/// Multi-tenant quota tracker.
///
/// Tenants are matched by **longest-prefix**: a write to
/// `/orgs/acme/k1` charges the tenant whose prefix is `/orgs/acme/`
/// if that prefix is registered (else the next-longest registered
/// ancestor, else the global default).
pub struct QuotaTracker {
    inner: Mutex<QuotaInner>,
}

#[derive(Debug, Default)]
struct QuotaInner {
    tenants: BTreeMap<Vec<u8>, TenantState>,
    default_limits: Option<QuotaLimits>,
    global_usage: UsageSnapshot,
}

impl Default for QuotaTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl QuotaTracker {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(QuotaInner::default()),
        }
    }

    pub fn set_default_limits(&self, l: QuotaLimits) {
        self.inner.lock().unwrap().default_limits = Some(l);
    }

    pub fn register_tenant(&self, prefix: impl Into<Vec<u8>>, limits: QuotaLimits) {
        let mut g = self.inner.lock().unwrap();
        g.tenants
            .entry(prefix.into())
            .or_default()
            .limits = Some(limits);
    }

    pub fn unregister_tenant(&self, prefix: &[u8]) -> Option<UsageSnapshot> {
        let mut g = self.inner.lock().unwrap();
        g.tenants.remove(prefix).map(|s| s.usage)
    }

    /// Match the longest registered prefix that is a prefix of `key`.
    fn match_prefix<'a>(g: &'a QuotaInner, key: &[u8]) -> Option<&'a Vec<u8>> {
        g.tenants
            .keys()
            .filter(|p| key.starts_with(p))
            .max_by_key(|p| p.len())
    }

    /// Decide whether a write of `bytes` to `key` should proceed.
    /// Does not mutate state.
    pub fn check_put(&self, key: &[u8], bytes: u64) -> QuotaDecision {
        let g = self.inner.lock().unwrap();
        let (usage, limits) = if let Some(prefix) = Self::match_prefix(&g, key) {
            let t = g.tenants.get(prefix).unwrap();
            (t.usage, t.limits.or(g.default_limits))
        } else {
            (g.global_usage, g.default_limits)
        };
        let Some(limits) = limits else {
            return QuotaDecision::Allow;
        };
        if usage.keys + 1 > limits.max_keys {
            return QuotaDecision::DenyMaxKeys {
                current: usage.keys,
                limit: limits.max_keys,
            };
        }
        if usage.bytes + bytes > limits.max_bytes {
            return QuotaDecision::DenyMaxBytes {
                current: usage.bytes,
                limit: limits.max_bytes,
            };
        }
        QuotaDecision::Allow
    }

    /// Record a successful put.
    pub fn record_put(&self, key: &[u8], bytes: u64) {
        let mut g = self.inner.lock().unwrap();
        g.global_usage.keys += 1;
        g.global_usage.bytes += bytes;
        let prefix = Self::match_prefix(&g, key).cloned();
        if let Some(prefix) = prefix {
            let t = g.tenants.get_mut(&prefix).unwrap();
            t.usage.keys += 1;
            t.usage.bytes += bytes;
        }
    }

    /// Record a successful delete of `bytes` for `key`. Underflows are
    /// clamped to zero so accounting cannot panic in production.
    pub fn record_delete(&self, key: &[u8], bytes: u64) {
        let mut g = self.inner.lock().unwrap();
        g.global_usage.keys = g.global_usage.keys.saturating_sub(1);
        g.global_usage.bytes = g.global_usage.bytes.saturating_sub(bytes);
        let prefix = Self::match_prefix(&g, key).cloned();
        if let Some(prefix) = prefix {
            let t = g.tenants.get_mut(&prefix).unwrap();
            t.usage.keys = t.usage.keys.saturating_sub(1);
            t.usage.bytes = t.usage.bytes.saturating_sub(bytes);
        }
    }

    pub fn usage_for(&self, prefix: &[u8]) -> UsageSnapshot {
        self.inner
            .lock()
            .unwrap()
            .tenants
            .get(prefix)
            .map(|t| t.usage)
            .unwrap_or_default()
    }

    pub fn global_usage(&self) -> UsageSnapshot {
        self.inner.lock().unwrap().global_usage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_when_no_limits_configured() {
        let q = QuotaTracker::new();
        assert!(q.check_put(b"/x", 1024).is_allow());
    }

    #[test]
    fn default_limits_apply_when_no_tenant_matches() {
        let q = QuotaTracker::new();
        q.set_default_limits(QuotaLimits::new(2, 100));
        q.record_put(b"/k1", 40);
        q.record_put(b"/k2", 40);
        let d = q.check_put(b"/k3", 1);
        assert!(matches!(d, QuotaDecision::DenyMaxKeys { current: 2, limit: 2 }));
    }

    #[test]
    fn tenant_byte_limit_enforced() {
        let q = QuotaTracker::new();
        q.register_tenant(b"/orgs/acme/".to_vec(), QuotaLimits::new(100, 1_000));
        q.record_put(b"/orgs/acme/k1", 600);
        let d = q.check_put(b"/orgs/acme/k2", 500);
        assert!(matches!(
            d,
            QuotaDecision::DenyMaxBytes {
                current: 600,
                limit: 1_000
            }
        ));
    }

    #[test]
    fn longest_prefix_wins() {
        let q = QuotaTracker::new();
        q.register_tenant(b"/orgs/".to_vec(), QuotaLimits::new(10, 10_000));
        q.register_tenant(b"/orgs/acme/".to_vec(), QuotaLimits::new(1, 10_000));
        q.record_put(b"/orgs/acme/k1", 1);
        let d = q.check_put(b"/orgs/acme/k2", 1);
        assert!(matches!(d, QuotaDecision::DenyMaxKeys { current: 1, limit: 1 }));
        // unrelated /orgs/ key is fine
        assert!(q.check_put(b"/orgs/other/k1", 1).is_allow());
    }

    #[test]
    fn delete_releases_capacity() {
        let q = QuotaTracker::new();
        q.register_tenant(b"/t/".to_vec(), QuotaLimits::new(2, 200));
        q.record_put(b"/t/a", 80);
        q.record_put(b"/t/b", 80);
        assert!(matches!(
            q.check_put(b"/t/c", 80),
            QuotaDecision::DenyMaxKeys { .. }
        ));
        q.record_delete(b"/t/a", 80);
        assert!(q.check_put(b"/t/c", 80).is_allow());
    }

    #[test]
    fn delete_does_not_underflow() {
        let q = QuotaTracker::new();
        q.register_tenant(b"/t/".to_vec(), QuotaLimits::new(10, 1_000));
        q.record_delete(b"/t/a", 9999); // never put, but must not panic
        assert_eq!(q.usage_for(b"/t/"), UsageSnapshot::default());
    }

    #[test]
    fn unregister_returns_final_usage() {
        let q = QuotaTracker::new();
        q.register_tenant(b"/t/".to_vec(), QuotaLimits::new(10, 1_000));
        q.record_put(b"/t/a", 7);
        let last = q.unregister_tenant(b"/t/").unwrap();
        assert_eq!(last, UsageSnapshot { keys: 1, bytes: 7 });
        // no tenant left → check_put falls back to global default
        assert!(q.check_put(b"/t/x", 1).is_allow());
    }

    #[test]
    fn global_usage_tracks_all_puts() {
        let q = QuotaTracker::new();
        q.record_put(b"/anywhere", 10);
        q.record_put(b"/elsewhere", 5);
        assert_eq!(q.global_usage(), UsageSnapshot { keys: 2, bytes: 15 });
    }

    #[test]
    fn unbounded_limits_never_deny() {
        let q = QuotaTracker::new();
        q.set_default_limits(QuotaLimits::unbounded());
        for _ in 0..1000 {
            q.record_put(b"/x", 1024);
        }
        assert!(q.check_put(b"/x", u64::MAX / 2).is_allow());
    }

    #[test]
    fn unregistered_prefix_charges_only_global() {
        let q = QuotaTracker::new();
        q.register_tenant(b"/orgs/acme/".to_vec(), QuotaLimits::new(10, 1_000));
        q.record_put(b"/orgs/other/x", 50); // does not charge acme
        assert_eq!(
            q.usage_for(b"/orgs/acme/"),
            UsageSnapshot::default()
        );
        assert_eq!(
            q.global_usage(),
            UsageSnapshot {
                keys: 1,
                bytes: 50
            }
        );
    }
}
