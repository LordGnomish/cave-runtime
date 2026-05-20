// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Quota enforcement.
//!
//! Upstream: `server/storage/quota/quota.go`.
//!
//! etcd has a single workspace-wide quota (`db-size-bytes`, default
//! 2 GiB) that fires the `NOSPACE` alarm when the backing bbolt
//! database grows past it. cave-etcd extends that with:
//!
//!   * per-tenant byte quotas (a tenant is a logical key prefix —
//!     `t/<tenant>/...`),
//!   * per-tenant key-count quotas,
//!   * a workspace-wide db-size byte quota that mirrors etcd's,
//!
//! all configurable at runtime. The module is pure logic with no I/O —
//! callers feed the current totals and the quota object returns a
//! `QuotaDecision` indicating whether the next write is allowed.
//!
//! The `BackendQuota` struct is `Send + Sync` so it can be wrapped in
//! `Arc` and shared across the axum router state.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A single quota verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaDecision {
    Allowed,
    /// Workspace-wide byte budget exhausted; etcd raises `NOSPACE`.
    NoSpace { db_bytes: u64, limit: u64 },
    /// Per-tenant byte budget exhausted.
    TenantBytes {
        tenant: String,
        bytes: u64,
        limit: u64,
    },
    /// Per-tenant key-count budget exhausted.
    TenantKeys {
        tenant: String,
        keys: u64,
        limit: u64,
    },
}

impl QuotaDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, QuotaDecision::Allowed)
    }
}

/// Per-tenant limits. `bytes` and `keys` are independent — either being
/// `0` means no limit on that dimension.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TenantLimits {
    pub bytes: u64,
    pub keys: u64,
}

/// Snapshot of usage at the time of a check. The caller fills this in
/// from real backend state (mvcc revision metadata, etc).
#[derive(Debug, Clone, Default)]
pub struct UsageSnapshot {
    pub db_bytes: u64,
    pub tenant_bytes: HashMap<String, u64>,
    pub tenant_keys: HashMap<String, u64>,
}

impl UsageSnapshot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_db_bytes(mut self, n: u64) -> Self {
        self.db_bytes = n;
        self
    }
    pub fn set_tenant(&mut self, tenant: impl Into<String>, bytes: u64, keys: u64) {
        let t = tenant.into();
        self.tenant_bytes.insert(t.clone(), bytes);
        self.tenant_keys.insert(t, keys);
    }
}

/// Backend quota config. Holds the workspace-wide limit and a
/// per-tenant table protected by an `RwLock` so the limits can be
/// updated without re-creating the quota.
pub struct BackendQuota {
    /// Workspace db-size limit in bytes; `0` means unlimited.
    db_size_bytes: RwLock<u64>,
    /// Per-tenant byte + key limits.
    tenants: RwLock<HashMap<String, TenantLimits>>,
}

/// Default `db-size-bytes` matches upstream etcd: 2 GiB.
pub const DEFAULT_DB_SIZE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

impl Default for BackendQuota {
    fn default() -> Self {
        Self::new(DEFAULT_DB_SIZE_BYTES)
    }
}

impl BackendQuota {
    pub fn new(db_size_bytes: u64) -> Self {
        Self {
            db_size_bytes: RwLock::new(db_size_bytes),
            tenants: RwLock::new(HashMap::new()),
        }
    }

    pub fn unlimited() -> Self {
        Self::new(0)
    }

    /// Replace the workspace-wide db-size byte limit (0 disables it).
    pub fn set_db_size_bytes(&self, n: u64) {
        *self.db_size_bytes.write().unwrap() = n;
    }

    /// Read the current workspace-wide limit.
    pub fn db_size_bytes(&self) -> u64 {
        *self.db_size_bytes.read().unwrap()
    }

    /// Install or replace a tenant's quota config.
    pub fn set_tenant_limits(&self, tenant: impl Into<String>, limits: TenantLimits) {
        self.tenants
            .write()
            .unwrap()
            .insert(tenant.into(), limits);
    }

    /// Remove a tenant from the table (no longer enforced).
    pub fn clear_tenant(&self, tenant: &str) -> bool {
        self.tenants.write().unwrap().remove(tenant).is_some()
    }

    /// Lookup a tenant's limits (returns the zero-value if unset).
    pub fn tenant_limits(&self, tenant: &str) -> TenantLimits {
        self.tenants
            .read()
            .unwrap()
            .get(tenant)
            .copied()
            .unwrap_or_default()
    }

    /// Total tenants tracked.
    pub fn tenant_count(&self) -> usize {
        self.tenants.read().unwrap().len()
    }

    /// Check whether a write that would bring the workspace + every
    /// listed tenant to the snapshot totals is permitted.
    ///
    /// The first failing dimension wins (db-size > tenant-bytes >
    /// tenant-keys) — mirrors etcd's `Quota.Available` ordering.
    pub fn check(&self, snap: &UsageSnapshot) -> QuotaDecision {
        let db_limit = self.db_size_bytes();
        if db_limit > 0 && snap.db_bytes > db_limit {
            return QuotaDecision::NoSpace {
                db_bytes: snap.db_bytes,
                limit: db_limit,
            };
        }
        let tenants = self.tenants.read().unwrap();
        // Stable iteration: lexicographic tenant ordering for
        // deterministic error reporting.
        let mut names: Vec<&String> = tenants.keys().collect();
        names.sort();
        for name in names {
            let limits = tenants.get(name).copied().unwrap_or_default();
            if limits.bytes > 0 {
                if let Some(b) = snap.tenant_bytes.get(name) {
                    if *b > limits.bytes {
                        return QuotaDecision::TenantBytes {
                            tenant: name.clone(),
                            bytes: *b,
                            limit: limits.bytes,
                        };
                    }
                }
            }
            if limits.keys > 0 {
                if let Some(k) = snap.tenant_keys.get(name) {
                    if *k > limits.keys {
                        return QuotaDecision::TenantKeys {
                            tenant: name.clone(),
                            keys: *k,
                            limit: limits.keys,
                        };
                    }
                }
            }
        }
        QuotaDecision::Allowed
    }

    /// Pre-write check: would adding `incoming_bytes` push us over the
    /// workspace cap, or push the named tenant over its byte cap?
    pub fn check_write(
        &self,
        snap: &UsageSnapshot,
        tenant: Option<&str>,
        incoming_bytes: u64,
    ) -> QuotaDecision {
        let db_limit = self.db_size_bytes();
        let projected_db = snap.db_bytes.saturating_add(incoming_bytes);
        if db_limit > 0 && projected_db > db_limit {
            return QuotaDecision::NoSpace {
                db_bytes: projected_db,
                limit: db_limit,
            };
        }
        if let Some(t) = tenant {
            let limits = self.tenant_limits(t);
            if limits.bytes > 0 {
                let current = snap.tenant_bytes.get(t).copied().unwrap_or(0);
                let projected = current.saturating_add(incoming_bytes);
                if projected > limits.bytes {
                    return QuotaDecision::TenantBytes {
                        tenant: t.to_string(),
                        bytes: projected,
                        limit: limits.bytes,
                    };
                }
            }
            if limits.keys > 0 {
                let current = snap.tenant_keys.get(t).copied().unwrap_or(0);
                let projected = current.saturating_add(1);
                if projected > limits.keys {
                    return QuotaDecision::TenantKeys {
                        tenant: t.to_string(),
                        keys: projected,
                        limit: limits.keys,
                    };
                }
            }
        }
        QuotaDecision::Allowed
    }
}

/// Shared-state alias — quota is typically held by axum routes via Arc.
pub type SharedQuota = Arc<BackendQuota>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_db_size_matches_upstream_two_gib() {
        let q = BackendQuota::default();
        assert_eq!(q.db_size_bytes(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn unlimited_quota_never_blocks_db_size() {
        let q = BackendQuota::unlimited();
        let snap = UsageSnapshot::new().with_db_bytes(u64::MAX / 2);
        assert_eq!(q.check(&snap), QuotaDecision::Allowed);
    }

    #[test]
    fn db_size_overflow_returns_nospace() {
        let q = BackendQuota::new(1000);
        let snap = UsageSnapshot::new().with_db_bytes(1500);
        let dec = q.check(&snap);
        assert_eq!(
            dec,
            QuotaDecision::NoSpace {
                db_bytes: 1500,
                limit: 1000
            }
        );
        assert!(!dec.is_allowed());
    }

    #[test]
    fn at_limit_is_still_allowed() {
        let q = BackendQuota::new(1000);
        let snap = UsageSnapshot::new().with_db_bytes(1000);
        assert_eq!(q.check(&snap), QuotaDecision::Allowed);
    }

    #[test]
    fn tenant_byte_overflow_blocks_writes() {
        let q = BackendQuota::unlimited();
        q.set_tenant_limits(
            "acme",
            TenantLimits {
                bytes: 100,
                keys: 0,
            },
        );
        let mut snap = UsageSnapshot::new();
        snap.set_tenant("acme", 150, 1);
        let dec = q.check(&snap);
        assert_eq!(
            dec,
            QuotaDecision::TenantBytes {
                tenant: "acme".to_string(),
                bytes: 150,
                limit: 100
            }
        );
    }

    #[test]
    fn tenant_key_overflow_blocks_writes() {
        let q = BackendQuota::unlimited();
        q.set_tenant_limits(
            "acme",
            TenantLimits {
                bytes: 0,
                keys: 5,
            },
        );
        let mut snap = UsageSnapshot::new();
        snap.set_tenant("acme", 10, 7);
        let dec = q.check(&snap);
        assert_eq!(
            dec,
            QuotaDecision::TenantKeys {
                tenant: "acme".to_string(),
                keys: 7,
                limit: 5
            }
        );
    }

    #[test]
    fn untracked_tenant_passes_through() {
        let q = BackendQuota::unlimited();
        q.set_tenant_limits(
            "acme",
            TenantLimits {
                bytes: 100,
                keys: 100,
            },
        );
        let mut snap = UsageSnapshot::new();
        snap.set_tenant("megacorp", u64::MAX / 4, u64::MAX / 4);
        assert_eq!(q.check(&snap), QuotaDecision::Allowed);
    }

    #[test]
    fn check_write_projects_incoming_bytes() {
        let q = BackendQuota::new(1000);
        let snap = UsageSnapshot::new().with_db_bytes(900);
        let dec = q.check_write(&snap, None, 200);
        assert_eq!(
            dec,
            QuotaDecision::NoSpace {
                db_bytes: 1100,
                limit: 1000
            }
        );
    }

    #[test]
    fn check_write_charges_to_tenant() {
        let q = BackendQuota::unlimited();
        q.set_tenant_limits(
            "t1",
            TenantLimits {
                bytes: 1000,
                keys: 100,
            },
        );
        let mut snap = UsageSnapshot::new();
        snap.set_tenant("t1", 950, 50);
        let dec = q.check_write(&snap, Some("t1"), 100);
        assert_eq!(
            dec,
            QuotaDecision::TenantBytes {
                tenant: "t1".to_string(),
                bytes: 1050,
                limit: 1000
            }
        );
    }

    #[test]
    fn check_write_increments_tenant_key_count() {
        let q = BackendQuota::unlimited();
        q.set_tenant_limits(
            "t1",
            TenantLimits {
                bytes: 0,
                keys: 10,
            },
        );
        let mut snap = UsageSnapshot::new();
        snap.set_tenant("t1", 0, 10);
        let dec = q.check_write(&snap, Some("t1"), 1);
        assert_eq!(
            dec,
            QuotaDecision::TenantKeys {
                tenant: "t1".to_string(),
                keys: 11,
                limit: 10
            }
        );
    }

    #[test]
    fn set_db_size_bytes_updates_limit_at_runtime() {
        let q = BackendQuota::new(1000);
        assert_eq!(q.db_size_bytes(), 1000);
        q.set_db_size_bytes(5000);
        assert_eq!(q.db_size_bytes(), 5000);
        let snap = UsageSnapshot::new().with_db_bytes(4000);
        assert_eq!(q.check(&snap), QuotaDecision::Allowed);
    }

    #[test]
    fn clear_tenant_drops_enforcement() {
        let q = BackendQuota::unlimited();
        q.set_tenant_limits(
            "doomed",
            TenantLimits {
                bytes: 1,
                keys: 0,
            },
        );
        let mut snap = UsageSnapshot::new();
        snap.set_tenant("doomed", 999, 1);
        assert!(!q.check(&snap).is_allowed());
        assert!(q.clear_tenant("doomed"));
        assert_eq!(q.check(&snap), QuotaDecision::Allowed);
        // clearing again is a no-op
        assert!(!q.clear_tenant("doomed"));
    }

    #[test]
    fn db_size_overflow_takes_precedence_over_tenant_overflow() {
        let q = BackendQuota::new(100);
        q.set_tenant_limits(
            "t",
            TenantLimits {
                bytes: 10,
                keys: 0,
            },
        );
        let mut snap = UsageSnapshot::new().with_db_bytes(500);
        snap.set_tenant("t", 500, 1);
        match q.check(&snap) {
            QuotaDecision::NoSpace { .. } => {}
            other => panic!("expected NoSpace, got {:?}", other),
        }
    }

    #[test]
    fn tenant_count_tracks_inserts_and_clears() {
        let q = BackendQuota::unlimited();
        assert_eq!(q.tenant_count(), 0);
        q.set_tenant_limits("a", TenantLimits::default());
        q.set_tenant_limits("b", TenantLimits::default());
        assert_eq!(q.tenant_count(), 2);
        q.clear_tenant("a");
        assert_eq!(q.tenant_count(), 1);
    }

    #[test]
    fn shared_quota_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SharedQuota>();
        let q: SharedQuota = Arc::new(BackendQuota::default());
        let _q2 = q.clone();
    }

    #[test]
    fn check_write_with_no_tenant_only_charges_db() {
        let q = BackendQuota::new(1000);
        let snap = UsageSnapshot::new().with_db_bytes(500);
        assert_eq!(q.check_write(&snap, None, 100), QuotaDecision::Allowed);
        assert_eq!(
            q.check_write(&snap, None, 600),
            QuotaDecision::NoSpace {
                db_bytes: 1100,
                limit: 1000
            }
        );
    }
}
