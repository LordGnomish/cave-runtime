// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Identity allocation coordinator — etcd-lock-based race protection.
//!
//! Mirrors `pkg/identity/cache/allocator.go::Allocator.AllocateIdentity`.
//! Multiple agents may try to allocate the same label set at the same
//! time; the coordinator uses a master-key lock in the KVStore to
//! ensure only one wins, with the others reading the result.
//!
//! Semantics (faithful to upstream):
//!
//! * `try_lock(key, owner, now)` — only the first owner wins; concurrent
//!   attempts return `LockHeld`.
//! * Lock has a TTL; if the holder doesn't `renew` before the TTL the
//!   lock is up for grabs again.
//! * `release(key, owner)` checks ownership before releasing.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockHolder {
    pub owner: String,
    pub acquired_ns: u64,
    pub expires_ns: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LockError {
    #[error("lock `{key}` is held by `{holder}` until {expires_ns}")]
    LockHeld { key: String, holder: String, expires_ns: u64 },
    #[error("lock `{key}` not held")]
    NotHeld { key: String },
    #[error("lock `{key}` is held by `{holder}` not `{requester}`")]
    NotOwner { key: String, holder: String, requester: String },
    #[error("tenant {tenant} cannot mutate lock store owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct IdentityLockCoordinator {
    pub tenant: TenantId,
    pub default_ttl_ns: u64,
    locks: HashMap<String, LockHolder>,
}

impl IdentityLockCoordinator {
    pub fn new(tenant: TenantId, default_ttl_seconds: u64) -> Self {
        Self {
            tenant,
            default_ttl_ns: default_ttl_seconds * 1_000_000_000,
            locks: HashMap::new(),
        }
    }

    pub fn try_lock(&mut self, key: impl Into<String>, owner: impl Into<String>, now_ns: u64) -> Result<LockHolder, LockError> {
        let key = key.into();
        let owner = owner.into();
        if let Some(h) = self.locks.get(&key) {
            if h.expires_ns > now_ns && h.owner != owner {
                return Err(LockError::LockHeld {
                    key, holder: h.owner.clone(), expires_ns: h.expires_ns,
                });
            }
        }
        let holder = LockHolder {
            owner, acquired_ns: now_ns,
            expires_ns: now_ns + self.default_ttl_ns,
        };
        self.locks.insert(key, holder.clone());
        Ok(holder)
    }

    pub fn renew(&mut self, key: &str, owner: &str, now_ns: u64) -> Result<LockHolder, LockError> {
        let h = self.locks.get_mut(key).ok_or_else(|| LockError::NotHeld { key: key.to_string() })?;
        if h.owner != owner {
            return Err(LockError::NotOwner { key: key.to_string(), holder: h.owner.clone(), requester: owner.to_string() });
        }
        h.expires_ns = now_ns + self.default_ttl_ns;
        Ok(h.clone())
    }

    pub fn release(&mut self, key: &str, owner: &str) -> Result<(), LockError> {
        let h = self.locks.get(key).ok_or_else(|| LockError::NotHeld { key: key.to_string() })?;
        if h.owner != owner {
            return Err(LockError::NotOwner { key: key.to_string(), holder: h.owner.clone(), requester: owner.to_string() });
        }
        self.locks.remove(key);
        Ok(())
    }

    pub fn holder(&self, key: &str) -> Option<&LockHolder> {
        self.locks.get(key)
    }

    pub fn len(&self) -> usize {
        self.locks.len()
    }

    /// Reap expired locks. Returns count removed.
    pub fn reap_expired(&mut self, now_ns: u64) -> usize {
        let before = self.locks.len();
        self.locks.retain(|_, h| h.expires_ns > now_ns);
        before - self.locks.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/identity/cache/allocator.go", "Allocator.AllocateIdentity");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn coord(tenant: TenantId) -> IdentityLockCoordinator {
        IdentityLockCoordinator::new(tenant, 30)
    }

    // ── try_lock ────────────────────────────────────────────────────────────

    #[test]
    fn try_lock_succeeds_when_unheld() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "TryLock", "tenant-id-tl");
        let mut c = coord(tenant);
        let h = c.try_lock("/cilium/identity/labels", "agent-a", 100).unwrap();
        assert_eq!(h.owner, "agent-a");
    }

    #[test]
    fn try_lock_held_by_other_within_ttl_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "TryLock.Held", "tenant-id-tlh");
        let mut c = coord(tenant);
        c.try_lock("/cilium/identity/labels", "agent-a", 100).unwrap();
        let err = c.try_lock("/cilium/identity/labels", "agent-b", 200).unwrap_err();
        match err {
            LockError::LockHeld { holder, .. } => assert_eq!(holder, "agent-a"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn try_lock_held_by_self_succeeds_renewing() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "TryLock.SameOwner", "tenant-id-tlso");
        let mut c = coord(tenant);
        c.try_lock("/cilium/identity/labels", "agent-a", 100).unwrap();
        let h = c.try_lock("/cilium/identity/labels", "agent-a", 200).unwrap();
        assert_eq!(h.acquired_ns, 200);
    }

    #[test]
    fn try_lock_after_ttl_lapsed_succeeds_for_other_owner() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "TryLock.AfterExpiry", "tenant-id-tlae");
        let mut c = coord(tenant);
        c.try_lock("/cilium/identity/labels", "agent-a", 100).unwrap();
        let h = c.try_lock("/cilium/identity/labels", "agent-b", 100 + 31_000_000_000).unwrap();
        assert_eq!(h.owner, "agent-b");
    }

    // ── renew ───────────────────────────────────────────────────────────────

    #[test]
    fn renew_extends_ttl_for_owner() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Renew", "tenant-id-rnw");
        let mut c = coord(tenant);
        let first = c.try_lock("/cilium/identity/labels", "agent-a", 100).unwrap();
        let renewed = c.renew("/cilium/identity/labels", "agent-a", 200).unwrap();
        assert!(renewed.expires_ns > first.expires_ns);
    }

    #[test]
    fn renew_by_non_owner_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Renew.NotOwner", "tenant-id-rno");
        let mut c = coord(tenant);
        c.try_lock("/cilium/identity/labels", "agent-a", 100).unwrap();
        let err = c.renew("/cilium/identity/labels", "agent-b", 200).unwrap_err();
        assert!(matches!(err, LockError::NotOwner { .. }));
    }

    #[test]
    fn renew_unheld_returns_not_held() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Renew.NotHeld", "tenant-id-rnh");
        let mut c = coord(tenant);
        let err = c.renew("/cilium/identity/labels", "agent-a", 100).unwrap_err();
        assert!(matches!(err, LockError::NotHeld { .. }));
    }

    // ── release ─────────────────────────────────────────────────────────────

    #[test]
    fn release_drops_lock_for_owner() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Release", "tenant-id-rel");
        let mut c = coord(tenant);
        c.try_lock("/cilium/identity/labels", "agent-a", 100).unwrap();
        c.release("/cilium/identity/labels", "agent-a").unwrap();
        assert!(c.holder("/cilium/identity/labels").is_none());
    }

    #[test]
    fn release_by_non_owner_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Release.NotOwner", "tenant-id-relno");
        let mut c = coord(tenant);
        c.try_lock("/cilium/identity/labels", "agent-a", 100).unwrap();
        let err = c.release("/cilium/identity/labels", "agent-b").unwrap_err();
        assert!(matches!(err, LockError::NotOwner { .. }));
    }

    #[test]
    fn release_unheld_returns_not_held() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Release.NotHeld", "tenant-id-relnh");
        let mut c = coord(tenant);
        let err = c.release("/cilium/identity/labels", "agent-a").unwrap_err();
        assert!(matches!(err, LockError::NotHeld { .. }));
    }

    // ── reap_expired ────────────────────────────────────────────────────────

    #[test]
    fn reap_expired_removes_locks_past_ttl() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Reap.Expired", "tenant-id-re");
        let mut c = coord(tenant);
        c.try_lock("a", "agent-a", 0).unwrap();
        c.try_lock("b", "agent-b", 0).unwrap();
        let n = c.reap_expired(31_000_000_000);
        assert_eq!(n, 2);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn reap_expired_keeps_fresh_locks() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Reap.Fresh", "tenant-id-rf");
        let mut c = coord(tenant);
        c.try_lock("a", "agent-a", 0).unwrap();
        let n = c.reap_expired(10_000_000_000);
        assert_eq!(n, 0);
        assert_eq!(c.len(), 1);
    }

    // ── Multi-key ──────────────────────────────────────────────────────────

    #[test]
    fn distinct_keys_are_independent() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "DistinctKeys", "tenant-id-dk");
        let mut c = coord(tenant);
        c.try_lock("a", "agent-a", 0).unwrap();
        c.try_lock("b", "agent-b", 0).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c.holder("a").unwrap().owner, "agent-a");
        assert_eq!(c.holder("b").unwrap().owner, "agent-b");
    }

    #[test]
    fn holder_returns_holder_struct() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Holder", "tenant-id-h");
        let mut c = coord(tenant);
        let h = c.try_lock("a", "agent-a", 100).unwrap();
        assert_eq!(c.holder("a"), Some(&h));
    }

    #[test]
    fn holder_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Holder.NotFound", "tenant-id-hnf");
        let c = coord(tenant);
        assert!(c.holder("ghost").is_none());
    }

    // ── Lock count ─────────────────────────────────────────────────────────

    #[test]
    fn len_tracks_lock_count() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Len", "tenant-id-len");
        let mut c = coord(tenant);
        for i in 0..5 {
            c.try_lock(format!("k-{i}"), "agent", 0).unwrap();
        }
        assert_eq!(c.len(), 5);
    }

    // ── Race scenarios ──────────────────────────────────────────────────────

    #[test]
    fn concurrent_lock_attempts_first_writer_wins() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Race.FirstWins", "tenant-id-fw");
        let mut c = coord(tenant);
        // Simulate two concurrent attempts at "the same time" by ordering.
        c.try_lock("/cilium/identity/labels/{app=web}", "agent-a", 100).unwrap();
        let err = c.try_lock("/cilium/identity/labels/{app=web}", "agent-b", 100).unwrap_err();
        assert!(matches!(err, LockError::LockHeld { .. }));
    }

    #[test]
    fn lock_renewal_preserves_owner() {
        let (_c, tenant) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Renew.Owner", "tenant-id-ro");
        let mut c = coord(tenant);
        c.try_lock("k", "agent-a", 0).unwrap();
        c.renew("k", "agent-a", 1000).unwrap();
        assert_eq!(c.holder("k").unwrap().owner, "agent-a");
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn lock_holder_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/cache/allocator.go", "Holder.Serde", "tenant-id-hserde");
        let h = LockHolder { owner: "agent-a".into(), acquired_ns: 100, expires_ns: 200 };
        let s = serde_json::to_string(&h).unwrap();
        let back: LockHolder = serde_json::from_str(&s).unwrap();
        assert_eq!(back, h);
    }
}
