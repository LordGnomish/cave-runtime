// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Leader election for the controller-manager itself.
//!
//! Mirrors upstream `kube-controller-manager`'s `--leader-elect=true`
//! mode: at most one controller-manager replica is the "active"
//! leader, the others stand by. The active leader is the only one
//! that runs reconciliation loops; standbys watch for the leader's
//! lease to expire and race to acquire it next.
//!
//! Distinct from `node_lease.rs`, which models **kube-node-lease**
//! (the per-kubelet liveness signal the controller-manager *watches*
//! to detect failed nodes). This module is the controller-manager's
//! *own* lease — the one that decides which replica drives the
//! reconciliation loops.
//!
//! Sweep-012 adoption: the lease primitive is
//! `cave_kernel::lease::LeaseManager`. Single-node MVP today (the
//! manager lives in-process); multi-node Raft-backed storage lands
//! with Paket C's consensus layer.
//!
//! Lifecycle a controller-manager replica follows:
//!
//! 1. **At startup**: call [`LeaderElector::acquire`] in a loop with
//!    a small retry delay. Becoming the leader means transitioning
//!    [`Role::Standby`] → [`Role::Leader`].
//! 2. **While leader**: tick [`LeaderElector::renew`] every
//!    `lease_ttl / 3` (matches the kube-controller-manager renew
//!    cadence in `pkg/leaderelection`).
//! 3. **On shutdown**: call [`LeaderElector::release`] so a standby
//!    can take over without waiting for the lease to expire.
//! 4. **As a standby**: poll [`LeaderElector::status`] periodically.
//!    When the lease expires, [`LeaderElector::acquire`] succeeds
//!    again and the standby promotes itself.

use cave_kernel::lease::{LeaseError, LeaseManager};
use std::time::{Duration, SystemTime};

/// Standard lease name for the controller-manager's election —
/// matches `kube-controller-manager`'s default `--leader-elect-resource-name`.
pub const CONTROLLER_MANAGER_LEASE: &str = "kube-controller-manager";

/// Recommended lease TTL — matches the upstream default
/// (`LeaderElectionConfiguration.LeaseDuration` = 15s).
pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(15);

/// Recommended renew interval — matches the upstream default
/// (`LeaderElectionConfiguration.RenewDeadline` = 10s, and we
/// renew at LeaseDuration / 3 ≈ 5s ahead of the deadline).
pub const DEFAULT_RENEW_INTERVAL: Duration = Duration::from_secs(5);

/// Replica's role in the election right now. Drives whether the
/// reconciliation loops should run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// This replica holds the lease — its reconcilers should run.
    Leader,
    /// Another replica holds the lease — this replica should stand
    /// by and watch for expiry.
    Standby,
}

/// Snapshot of the election state. Useful for the admin UI + tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElectionStatus {
    pub role: Role,
    pub current_holder: Option<String>,
    pub expires_at_unix: Option<u64>,
    pub revision: Option<u64>,
}

/// Leader-elector handle for one controller-manager replica.
#[derive(Debug, Clone)]
pub struct LeaderElector {
    manager: LeaseManager,
    lease_name: String,
    replica_id: String,
    lease_ttl: Duration,
}

impl LeaderElector {
    pub fn new(
        manager: LeaseManager,
        lease_name: impl Into<String>,
        replica_id: impl Into<String>,
        lease_ttl: Duration,
    ) -> Self {
        Self {
            manager,
            lease_name: lease_name.into(),
            replica_id: replica_id.into(),
            lease_ttl,
        }
    }

    /// Build a [`LeaderElector`] with the upstream-default
    /// `kube-controller-manager` lease name and TTL.
    pub fn default_for_replica(manager: LeaseManager, replica_id: impl Into<String>) -> Self {
        Self::new(manager, CONTROLLER_MANAGER_LEASE, replica_id, DEFAULT_LEASE_TTL)
    }

    pub fn replica_id(&self) -> &str { &self.replica_id }
    pub fn lease_name(&self) -> &str { &self.lease_name }
    pub fn lease_ttl(&self) -> Duration { self.lease_ttl }

    /// Attempt to become the leader. Succeeds in two cases:
    ///
    /// * The lease is free (no previous leader, or the previous
    ///   leader's lease has expired).
    /// * This replica was already the leader (re-acquisition slides
    ///   the expiry forward — same as etcd's `KeepAlive`).
    ///
    /// Returns the new [`Role`]: `Leader` on success, `Standby` if
    /// another replica still holds an unexpired lease.
    pub fn acquire(&self, now: SystemTime) -> Role {
        match self.manager.acquire(&self.lease_name, &self.replica_id, self.lease_ttl, now) {
            Ok(_) => Role::Leader,
            Err(LeaseError::Held { .. }) => Role::Standby,
            Err(LeaseError::InvalidTtl) | Err(LeaseError::NotFound(_)) => Role::Standby,
        }
    }

    /// Renew the lease as the current leader. Returns `Ok(())` on
    /// success, `Err(LeaseError)` if this replica no longer holds
    /// the lease (someone else acquired it — the replica should
    /// step down to standby and stop reconcilers immediately).
    pub fn renew(&self, now: SystemTime) -> Result<(), LeaseError> {
        self.manager.renew(&self.lease_name, &self.replica_id, self.lease_ttl, now)?;
        Ok(())
    }

    /// Voluntarily release the lease — call this on graceful
    /// shutdown so the next replica can take over without waiting
    /// for the lease to expire. Idempotent: silently succeeds if
    /// the lease is already gone.
    pub fn release(&self) -> Result<(), LeaseError> {
        match self.manager.revoke(&self.lease_name, &self.replica_id) {
            Ok(()) => Ok(()),
            Err(LeaseError::NotFound(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Snapshot the current election state without mutating it.
    pub fn status(&self, now: SystemTime) -> ElectionStatus {
        let info = self.manager.get(&self.lease_name);
        let now_unix = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match info {
            Some(i) if i.expires_at_unix > now_unix => {
                let role = if i.holder == self.replica_id { Role::Leader } else { Role::Standby };
                ElectionStatus {
                    role,
                    current_holder: Some(i.holder),
                    expires_at_unix: Some(i.expires_at_unix),
                    revision: Some(i.revision),
                }
            }
            _ => ElectionStatus {
                role: Role::Standby,
                current_holder: None,
                expires_at_unix: None,
                revision: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds_since_epoch: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds_since_epoch)
    }

    #[test]
    fn acquire_promotes_a_free_lease_to_leader() {
        let mgr = LeaseManager::new();
        let e = LeaderElector::default_for_replica(mgr, "ctrlmgr-1");
        assert_eq!(e.acquire(at(1_000_000)), Role::Leader);
    }

    #[test]
    fn second_replica_acquiring_unexpired_lease_stays_standby() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::default_for_replica(mgr.clone(), "ctrlmgr-a");
        let b = LeaderElector::default_for_replica(mgr, "ctrlmgr-b");
        assert_eq!(a.acquire(at(1_000_000)), Role::Leader);
        assert_eq!(b.acquire(at(1_000_001)), Role::Standby);
    }

    #[test]
    fn standby_takes_over_after_lease_expires() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::new(mgr.clone(), "el", "a", Duration::from_secs(5));
        let b = LeaderElector::new(mgr, "el", "b", Duration::from_secs(5));
        assert_eq!(a.acquire(at(1_000_000)), Role::Leader);
        // While the lease is still held (`expires_at > now`, strict),
        // standby can't take over.
        assert_eq!(b.acquire(at(1_000_004)), Role::Standby);
        // Exactly at expiry the kernel's `expires_at > now` check is
        // false, so the lease is considered expired and the standby
        // acquires.
        assert_eq!(b.acquire(at(1_000_005)), Role::Leader);
    }

    #[test]
    fn renew_succeeds_for_current_leader_and_slides_expiry() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::default_for_replica(mgr.clone(), "a");
        a.acquire(at(1_000_000));
        // 5s later — renew before lease ages.
        a.renew(at(1_000_005)).unwrap();
        let s = a.status(at(1_000_005));
        assert_eq!(s.role, Role::Leader);
        // Expiry slid to 1_000_005 + 15 = 1_000_020 (default TTL 15s).
        assert_eq!(s.expires_at_unix, Some(1_000_020));
    }

    #[test]
    fn renew_fails_for_non_leader() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::default_for_replica(mgr.clone(), "a");
        let b = LeaderElector::default_for_replica(mgr, "b");
        a.acquire(at(1_000_000));
        assert!(b.renew(at(1_000_001)).is_err());
    }

    #[test]
    fn release_lets_standby_acquire_immediately() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::default_for_replica(mgr.clone(), "a");
        let b = LeaderElector::default_for_replica(mgr, "b");
        a.acquire(at(1_000_000));
        a.release().unwrap();
        assert_eq!(b.acquire(at(1_000_001)), Role::Leader);
    }

    #[test]
    fn release_is_idempotent_when_lease_already_gone() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::default_for_replica(mgr, "a");
        a.acquire(at(1_000_000));
        a.release().unwrap();
        a.release().unwrap();
    }

    #[test]
    fn status_reports_leader_when_we_hold_unexpired_lease() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::default_for_replica(mgr, "ctrlmgr-1");
        a.acquire(at(1_000_000));
        let s = a.status(at(1_000_001));
        assert_eq!(s.role, Role::Leader);
        assert_eq!(s.current_holder.as_deref(), Some("ctrlmgr-1"));
        assert!(s.expires_at_unix.unwrap() > 1_000_001);
    }

    #[test]
    fn status_reports_standby_when_other_holds_lease() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::default_for_replica(mgr.clone(), "a");
        let b = LeaderElector::default_for_replica(mgr, "b");
        a.acquire(at(1_000_000));
        let s = b.status(at(1_000_001));
        assert_eq!(s.role, Role::Standby);
        assert_eq!(s.current_holder.as_deref(), Some("a"));
    }

    #[test]
    fn status_reports_standby_with_no_holder_after_expiry() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::new(mgr, "el", "a", Duration::from_secs(2));
        a.acquire(at(1_000_000));
        let s = a.status(at(1_000_010));
        assert_eq!(s.role, Role::Standby);
        assert!(s.current_holder.is_none());
    }

    #[test]
    fn re_acquire_by_same_replica_slides_expiry_no_role_flip() {
        let mgr = LeaseManager::new();
        let a = LeaderElector::default_for_replica(mgr, "a");
        assert_eq!(a.acquire(at(1_000_000)), Role::Leader);
        // Re-acquiring while we still hold it must keep us leader
        // and slide the expiry forward.
        assert_eq!(a.acquire(at(1_000_007)), Role::Leader);
        let s = a.status(at(1_000_007));
        assert_eq!(s.expires_at_unix, Some(1_000_022));
    }

    #[test]
    fn default_for_replica_uses_upstream_constants() {
        let mgr = LeaseManager::new();
        let e = LeaderElector::default_for_replica(mgr, "x");
        assert_eq!(e.lease_name(), CONTROLLER_MANAGER_LEASE);
        assert_eq!(e.lease_ttl(), DEFAULT_LEASE_TTL);
    }

    #[test]
    fn replica_id_accessor_returns_constructor_value() {
        let mgr = LeaseManager::new();
        let e = LeaderElector::default_for_replica(mgr, "ctrlmgr-7");
        assert_eq!(e.replica_id(), "ctrlmgr-7");
    }
}
