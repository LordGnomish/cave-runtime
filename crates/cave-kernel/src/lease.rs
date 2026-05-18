// SPDX-License-Identifier: AGPL-3.0-or-later
//! Distributed-lease primitive — etcd-style leader-election lease
//! with renewal + expiry. Adopters use this to implement
//! "exactly one controller is the active leader at any moment"
//! semantics (cave-controller-manager, cave-rdbms-operator).
//!
//! Single-node MVP: leases live in an in-memory `LeaseManager`
//! keyed by a string lease name. Renewals slide the expiry forward;
//! `acquire()` only succeeds if the existing lease has expired OR
//! belongs to the caller. The shape is faithful to etcd's
//! `etcdserverpb.Lease{Grant,Revoke,KeepAlive}` API so the future
//! multi-node port can swap the storage backend without changing
//! the caller-facing types.
//!
//! Adopters: cave-controller-manager (leader election),
//! cave-rdbms-operator (primary-election fencing).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// One held lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseInfo {
    pub name: String,
    pub holder: String,
    /// Absolute expiry time as unix seconds.
    pub expires_at_unix: u64,
    /// Monotonically increasing across acquires/renewals — etcd's
    /// "revision" concept. Lets a caller detect lease takeovers.
    pub revision: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("lease {name} held by {holder} until unix {expires_at}")]
    Held { name: String, holder: String, expires_at: u64 },
    #[error("lease {0} does not exist")]
    NotFound(String),
    #[error("ttl_seconds must be > 0")]
    InvalidTtl,
}

/// In-memory lease manager. Cheap to clone — backed by `Arc<RwLock>`.
#[derive(Debug, Clone, Default)]
pub struct LeaseManager {
    inner: Arc<RwLock<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    leases: HashMap<String, LeaseInfo>,
    next_revision: u64,
}

impl LeaseManager {
    pub fn new() -> Self { Self::default() }

    /// Acquire `name` for `holder` with the given TTL. Succeeds if
    /// the lease is free OR currently held by the same `holder`
    /// (re-acquisition slides the expiry forward, same as etcd's
    /// `KeepAlive`). Fails when a different holder has an
    /// unexpired lease.
    pub fn acquire(
        &self,
        name: &str,
        holder: &str,
        ttl: Duration,
        now: SystemTime,
    ) -> Result<LeaseInfo, LeaseError> {
        if ttl.is_zero() { return Err(LeaseError::InvalidTtl); }
        let now_unix = unix_seconds(now);
        let mut g = self.inner.write().unwrap();
        if let Some(existing) = g.leases.get(name) {
            if existing.expires_at_unix > now_unix && existing.holder != holder {
                return Err(LeaseError::Held {
                    name: name.into(),
                    holder: existing.holder.clone(),
                    expires_at: existing.expires_at_unix,
                });
            }
        }
        g.next_revision += 1;
        let info = LeaseInfo {
            name: name.into(),
            holder: holder.into(),
            expires_at_unix: now_unix.saturating_add(ttl.as_secs()),
            revision: g.next_revision,
        };
        g.leases.insert(name.into(), info.clone());
        Ok(info)
    }

    /// Renew an existing lease the caller already holds. Slides
    /// `expires_at` forward by `ttl`. Fails if the lease doesn't
    /// exist or is held by someone else.
    pub fn renew(
        &self,
        name: &str,
        holder: &str,
        ttl: Duration,
        now: SystemTime,
    ) -> Result<LeaseInfo, LeaseError> {
        if ttl.is_zero() { return Err(LeaseError::InvalidTtl); }
        let mut g = self.inner.write().unwrap();
        let info = g.leases.get_mut(name)
            .ok_or_else(|| LeaseError::NotFound(name.into()))?;
        if info.holder != holder {
            return Err(LeaseError::Held {
                name: name.into(),
                holder: info.holder.clone(),
                expires_at: info.expires_at_unix,
            });
        }
        info.expires_at_unix = unix_seconds(now).saturating_add(ttl.as_secs());
        Ok(info.clone())
    }

    /// Explicit revoke — equivalent to etcd's `LeaseRevoke`. Only
    /// the current holder may revoke.
    pub fn revoke(&self, name: &str, holder: &str) -> Result<(), LeaseError> {
        let mut g = self.inner.write().unwrap();
        let info = g.leases.get(name)
            .ok_or_else(|| LeaseError::NotFound(name.into()))?;
        if info.holder != holder {
            return Err(LeaseError::Held {
                name: name.into(),
                holder: info.holder.clone(),
                expires_at: info.expires_at_unix,
            });
        }
        g.leases.remove(name);
        Ok(())
    }

    /// Look up a lease by name. Returns the lease info regardless of
    /// expiry — callers compare against their own clock to decide if
    /// it's still valid.
    pub fn get(&self, name: &str) -> Option<LeaseInfo> {
        self.inner.read().unwrap().leases.get(name).cloned()
    }

    /// Snapshot of every active lease — useful for the dashboard.
    pub fn list(&self) -> Vec<LeaseInfo> {
        self.inner.read().unwrap().leases.values().cloned().collect()
    }

    /// Sweep expired leases. Returns the count removed.
    pub fn sweep_expired(&self, now: SystemTime) -> usize {
        let now_unix = unix_seconds(now);
        let mut g = self.inner.write().unwrap();
        let before = g.leases.len();
        g.leases.retain(|_, info| info.expires_at_unix > now_unix);
        before - g.leases.len()
    }
}

fn unix_seconds(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> SystemTime { SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000) }

    #[test]
    fn acquire_succeeds_on_free_lease() {
        let m = LeaseManager::new();
        let info = m.acquire("leader", "node-a", Duration::from_secs(30), now()).unwrap();
        assert_eq!(info.name, "leader");
        assert_eq!(info.holder, "node-a");
        assert_eq!(info.expires_at_unix, 1_000_030);
        assert_eq!(info.revision, 1);
    }

    #[test]
    fn acquire_fails_when_other_holds_unexpired_lease() {
        let m = LeaseManager::new();
        m.acquire("leader", "node-a", Duration::from_secs(30), now()).unwrap();
        let err = m.acquire("leader", "node-b", Duration::from_secs(30), now()).unwrap_err();
        match err {
            LeaseError::Held { holder, expires_at, .. } => {
                assert_eq!(holder, "node-a");
                assert_eq!(expires_at, 1_000_030);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn acquire_same_holder_slides_expiry_and_bumps_revision() {
        let m = LeaseManager::new();
        let v1 = m.acquire("l", "n", Duration::from_secs(5), now()).unwrap();
        let v2 = m.acquire("l", "n", Duration::from_secs(60), now()).unwrap();
        assert_eq!(v2.holder, "n");
        assert_eq!(v2.expires_at_unix, 1_000_060);
        assert!(v2.revision > v1.revision);
    }

    #[test]
    fn acquire_after_expiry_succeeds_for_new_holder() {
        let m = LeaseManager::new();
        m.acquire("l", "old", Duration::from_secs(5), now()).unwrap();
        let later = now() + Duration::from_secs(10);
        let v = m.acquire("l", "new", Duration::from_secs(5), later).unwrap();
        assert_eq!(v.holder, "new");
    }

    #[test]
    fn renew_slides_expiry_for_current_holder() {
        let m = LeaseManager::new();
        let v1 = m.acquire("l", "n", Duration::from_secs(5), now()).unwrap();
        let later = now() + Duration::from_secs(2);
        let v2 = m.renew("l", "n", Duration::from_secs(60), later).unwrap();
        assert_eq!(v2.expires_at_unix, 1_000_000 + 2 + 60);
        assert!(v2.revision == v1.revision); // renew doesn't bump revision
    }

    #[test]
    fn renew_fails_for_non_holder() {
        let m = LeaseManager::new();
        m.acquire("l", "a", Duration::from_secs(5), now()).unwrap();
        assert!(matches!(
            m.renew("l", "b", Duration::from_secs(5), now()).unwrap_err(),
            LeaseError::Held { .. }
        ));
    }

    #[test]
    fn renew_fails_for_missing_lease() {
        let m = LeaseManager::new();
        assert!(matches!(
            m.renew("nope", "a", Duration::from_secs(1), now()).unwrap_err(),
            LeaseError::NotFound(_)
        ));
    }

    #[test]
    fn revoke_removes_lease_for_holder() {
        let m = LeaseManager::new();
        m.acquire("l", "a", Duration::from_secs(5), now()).unwrap();
        m.revoke("l", "a").unwrap();
        assert!(m.get("l").is_none());
    }

    #[test]
    fn revoke_fails_for_non_holder() {
        let m = LeaseManager::new();
        m.acquire("l", "a", Duration::from_secs(5), now()).unwrap();
        assert!(matches!(
            m.revoke("l", "b").unwrap_err(),
            LeaseError::Held { .. }
        ));
    }

    #[test]
    fn invalid_ttl_rejected_at_acquire_and_renew() {
        let m = LeaseManager::new();
        assert!(matches!(
            m.acquire("l", "a", Duration::ZERO, now()).unwrap_err(),
            LeaseError::InvalidTtl
        ));
        m.acquire("l", "a", Duration::from_secs(1), now()).unwrap();
        assert!(matches!(
            m.renew("l", "a", Duration::ZERO, now()).unwrap_err(),
            LeaseError::InvalidTtl
        ));
    }

    #[test]
    fn list_returns_all_active_leases() {
        let m = LeaseManager::new();
        m.acquire("a", "n", Duration::from_secs(1), now()).unwrap();
        m.acquire("b", "n", Duration::from_secs(1), now()).unwrap();
        let l = m.list();
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn sweep_expired_removes_only_expired() {
        let m = LeaseManager::new();
        m.acquire("a", "n", Duration::from_secs(5), now()).unwrap();
        m.acquire("b", "n", Duration::from_secs(60), now()).unwrap();
        let later = now() + Duration::from_secs(10);
        let removed = m.sweep_expired(later);
        assert_eq!(removed, 1);
        assert!(m.get("a").is_none());
        assert!(m.get("b").is_some());
    }

    #[test]
    fn manager_clone_shares_storage() {
        let m = LeaseManager::new();
        let m2 = m.clone();
        m.acquire("l", "n", Duration::from_secs(1), now()).unwrap();
        assert!(m2.get("l").is_some());
    }

    #[test]
    fn revisions_are_monotonic_across_acquires() {
        let m = LeaseManager::new();
        let v1 = m.acquire("a", "x", Duration::from_secs(1), now()).unwrap();
        let v2 = m.acquire("b", "x", Duration::from_secs(1), now()).unwrap();
        assert!(v2.revision > v1.revision);
    }
}
