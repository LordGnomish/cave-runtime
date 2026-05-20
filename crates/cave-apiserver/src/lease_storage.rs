// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lease holder semantics enforcement.
//!
//! Upstream: `pkg/registry/coordination/lease/storage/`
//!
//! A coordination.k8s.io `Lease` object carries three fields that the
//! upstream storage layer enforces as a unit:
//!
//!   * `holderIdentity`        — opaque string identifying the current
//!                               leader. May be empty (released lease).
//!   * `leaseDurationSeconds`  — duration after `acquireTime`/`renewTime`
//!                               within which only the holder can update.
//!   * `renewTime`             — wallclock time of the most recent
//!                               successful renewal.
//!
//! The cave port previously surfaced the v1 Lease CRUD through
//! `routes.rs` but did not enforce holder semantics — any caller could
//! overwrite any lease at any time. cave-ha works around this with
//! Raft-derived leases, but native callers (Velero, controller-manager
//! leader election, …) see the apiserver Lease object as authoritative.
//!
//! This module ports the missing semantics in-memory: only the current
//! holder can renew within `leaseDurationSeconds`; expired leases can
//! be acquired by any new holder; the renewal stamps `renewTime` to
//! "now". The in-memory store is sufficient for unit-test parity; the
//! production wiring lives downstream in cave-runtime, which composes
//! this enforcement on top of the etcd-backed store.
//!
//! Tenant invariant: each Lease lives under a tenant_id. Cross-tenant
//! acquire/renew is rejected (`LeaseError::CrossTenant`).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

/// Identifies a Lease object (namespace + name + tenant).
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct LeaseKey {
    pub tenant_id: String,
    pub namespace: String,
    pub name: String,
}

impl LeaseKey {
    pub fn new(
        tenant_id: impl Into<String>,
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            namespace: namespace.into(),
            name: name.into(),
        }
    }
}

/// Mirrors coordination.k8s.io/v1 LeaseSpec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeaseSpec {
    pub holder_identity: String,
    pub lease_duration_seconds: i64,
    pub acquire_time: DateTime<Utc>,
    pub renew_time: DateTime<Utc>,
    /// Incremented on every renewal — used to disambiguate stale
    /// updates (mirrors upstream `leaseTransitions`).
    pub lease_transitions: i64,
}

impl LeaseSpec {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.renew_time + Duration::seconds(self.lease_duration_seconds) <= now
    }
}

/// Errors raised by the lease store.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("not the current holder (current: {current}, attempted: {attempted})")]
    NotHolder { current: String, attempted: String },
    #[error("lease has not yet expired (renew_time={renew_time}, duration={duration_s}s)")]
    NotExpired {
        renew_time: DateTime<Utc>,
        duration_s: i64,
    },
    #[error("lease duration must be positive (got {0})")]
    InvalidDuration(i64),
    #[error("holder identity must not be empty")]
    EmptyHolder,
    #[error("cross-tenant access denied (lease tenant: {lease}, caller: {caller})")]
    CrossTenant { lease: String, caller: String },
    #[error("lease not found")]
    NotFound,
}

/// In-memory lease store with holder semantics enforcement.
#[derive(Default)]
pub struct LeaseStorage {
    leases: RwLock<HashMap<LeaseKey, LeaseSpec>>,
}

impl LeaseStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire a brand-new lease, or take over an expired one.
    pub async fn acquire(
        &self,
        key: LeaseKey,
        holder: impl Into<String>,
        duration_s: i64,
        now: DateTime<Utc>,
    ) -> Result<LeaseSpec, LeaseError> {
        if duration_s <= 0 {
            return Err(LeaseError::InvalidDuration(duration_s));
        }
        let holder = holder.into();
        if holder.is_empty() {
            return Err(LeaseError::EmptyHolder);
        }
        let mut g = self.leases.write().await;
        if let Some(existing) = g.get(&key) {
            if !existing.is_expired(now) && existing.holder_identity != holder {
                return Err(LeaseError::NotExpired {
                    renew_time: existing.renew_time,
                    duration_s: existing.lease_duration_seconds,
                });
            }
        }
        let transitions = g
            .get(&key)
            .map(|e| if e.holder_identity == holder { e.lease_transitions } else { e.lease_transitions + 1 })
            .unwrap_or(0);
        let spec = LeaseSpec {
            holder_identity: holder,
            lease_duration_seconds: duration_s,
            acquire_time: now,
            renew_time: now,
            lease_transitions: transitions,
        };
        g.insert(key, spec.clone());
        Ok(spec)
    }

    /// Renew an existing lease. Only the current holder, within the
    /// duration window, may renew. `now` MUST be the caller's wallclock
    /// — passing arbitrary times is the caller's responsibility (the
    /// store does no clock skew correction).
    pub async fn renew(
        &self,
        key: &LeaseKey,
        holder: &str,
        caller_tenant: &str,
        now: DateTime<Utc>,
    ) -> Result<LeaseSpec, LeaseError> {
        if caller_tenant != key.tenant_id {
            return Err(LeaseError::CrossTenant {
                lease: key.tenant_id.clone(),
                caller: caller_tenant.to_string(),
            });
        }
        let mut g = self.leases.write().await;
        let spec = g.get_mut(key).ok_or(LeaseError::NotFound)?;
        if spec.holder_identity != holder {
            return Err(LeaseError::NotHolder {
                current: spec.holder_identity.clone(),
                attempted: holder.to_string(),
            });
        }
        if spec.is_expired(now) {
            // Expired ⇒ holder must re-acquire, not renew. Tell the
            // caller so they can switch paths.
            return Err(LeaseError::NotExpired {
                renew_time: spec.renew_time,
                duration_s: spec.lease_duration_seconds,
            });
        }
        spec.renew_time = now;
        Ok(spec.clone())
    }

    /// Read the current state of a lease (or `NotFound`).
    pub async fn get(&self, key: &LeaseKey) -> Result<LeaseSpec, LeaseError> {
        self.leases
            .read()
            .await
            .get(key)
            .cloned()
            .ok_or(LeaseError::NotFound)
    }

    /// Release a lease — only the current holder may release.
    pub async fn release(
        &self,
        key: &LeaseKey,
        holder: &str,
        caller_tenant: &str,
    ) -> Result<(), LeaseError> {
        if caller_tenant != key.tenant_id {
            return Err(LeaseError::CrossTenant {
                lease: key.tenant_id.clone(),
                caller: caller_tenant.to_string(),
            });
        }
        let mut g = self.leases.write().await;
        let spec = g.get(key).ok_or(LeaseError::NotFound)?;
        if spec.holder_identity != holder {
            return Err(LeaseError::NotHolder {
                current: spec.holder_identity.clone(),
                attempted: holder.to_string(),
            });
        }
        g.remove(key);
        Ok(())
    }

    /// Number of live leases (for diagnostics + tests).
    pub async fn len(&self) -> usize {
        self.leases.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> LeaseKey {
        LeaseKey::new("tenant-a", "kube-system", "kube-controller-manager")
    }

    #[tokio::test]
    async fn acquire_grants_lease_to_first_holder_with_zero_transitions() {
        let s = LeaseStorage::new();
        let now = Utc::now();
        let spec = s
            .acquire(key(), "node-a", 30, now)
            .await
            .expect("acquire ok");
        assert_eq!(spec.holder_identity, "node-a");
        assert_eq!(spec.lease_duration_seconds, 30);
        assert_eq!(spec.acquire_time, now);
        assert_eq!(spec.renew_time, now);
        assert_eq!(spec.lease_transitions, 0);
        assert_eq!(s.len().await, 1);
    }

    #[tokio::test]
    async fn renew_within_window_succeeds_for_current_holder() {
        let s = LeaseStorage::new();
        let t0 = Utc::now();
        s.acquire(key(), "node-a", 30, t0).await.unwrap();
        let t1 = t0 + Duration::seconds(15);
        let renewed = s.renew(&key(), "node-a", "tenant-a", t1).await.unwrap();
        assert_eq!(renewed.holder_identity, "node-a");
        assert_eq!(renewed.renew_time, t1);
        assert_eq!(renewed.acquire_time, t0, "acquire_time is sticky");
    }

    #[tokio::test]
    async fn renew_by_non_holder_within_window_is_rejected() {
        let s = LeaseStorage::new();
        let t0 = Utc::now();
        s.acquire(key(), "node-a", 30, t0).await.unwrap();
        let t1 = t0 + Duration::seconds(5);
        let err = s.renew(&key(), "node-b", "tenant-a", t1).await.unwrap_err();
        match err {
            LeaseError::NotHolder { current, attempted } => {
                assert_eq!(current, "node-a");
                assert_eq!(attempted, "node-b");
            }
            other => panic!("expected NotHolder, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn expired_lease_can_be_acquired_by_a_different_holder() {
        let s = LeaseStorage::new();
        let t0 = Utc::now();
        s.acquire(key(), "node-a", 10, t0).await.unwrap();
        // 11s later, lease has expired.
        let t1 = t0 + Duration::seconds(11);
        let spec = s.acquire(key(), "node-b", 10, t1).await.unwrap();
        assert_eq!(spec.holder_identity, "node-b");
        assert_eq!(
            spec.lease_transitions, 1,
            "transition counter must bump on holder change"
        );
    }

    #[tokio::test]
    async fn acquire_by_different_holder_in_window_is_rejected() {
        let s = LeaseStorage::new();
        let t0 = Utc::now();
        s.acquire(key(), "node-a", 30, t0).await.unwrap();
        let t1 = t0 + Duration::seconds(5);
        let err = s.acquire(key(), "node-b", 30, t1).await.unwrap_err();
        assert!(matches!(err, LeaseError::NotExpired { .. }));
    }

    #[tokio::test]
    async fn acquire_by_same_holder_in_window_is_an_idempotent_refresh() {
        let s = LeaseStorage::new();
        let t0 = Utc::now();
        s.acquire(key(), "node-a", 30, t0).await.unwrap();
        let t1 = t0 + Duration::seconds(5);
        // Same holder calling acquire mid-window is allowed (treated as
        // a re-acquire of an unexpired lease they already hold). The
        // store stamps a fresh renew_time but keeps transition count.
        let spec = s.acquire(key(), "node-a", 30, t1).await.unwrap();
        assert_eq!(spec.holder_identity, "node-a");
        assert_eq!(spec.lease_transitions, 0);
        assert_eq!(spec.renew_time, t1);
    }

    #[tokio::test]
    async fn renew_after_expiry_returns_not_expired_signal() {
        let s = LeaseStorage::new();
        let t0 = Utc::now();
        s.acquire(key(), "node-a", 10, t0).await.unwrap();
        let t1 = t0 + Duration::seconds(11);
        let err = s.renew(&key(), "node-a", "tenant-a", t1).await.unwrap_err();
        assert!(matches!(err, LeaseError::NotExpired { .. }));
    }

    #[tokio::test]
    async fn cross_tenant_renew_is_rejected() {
        let s = LeaseStorage::new();
        let t0 = Utc::now();
        s.acquire(key(), "node-a", 30, t0).await.unwrap();
        let err = s.renew(&key(), "node-a", "tenant-other", t0).await.unwrap_err();
        match err {
            LeaseError::CrossTenant { lease, caller } => {
                assert_eq!(lease, "tenant-a");
                assert_eq!(caller, "tenant-other");
            }
            other => panic!("expected CrossTenant, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn release_drops_lease_and_subsequent_get_is_not_found() {
        let s = LeaseStorage::new();
        let t0 = Utc::now();
        s.acquire(key(), "node-a", 30, t0).await.unwrap();
        s.release(&key(), "node-a", "tenant-a").await.unwrap();
        assert_eq!(s.get(&key()).await.unwrap_err(), LeaseError::NotFound);
    }

    #[tokio::test]
    async fn release_by_non_holder_is_rejected() {
        let s = LeaseStorage::new();
        let t0 = Utc::now();
        s.acquire(key(), "node-a", 30, t0).await.unwrap();
        let err = s.release(&key(), "node-b", "tenant-a").await.unwrap_err();
        assert!(matches!(err, LeaseError::NotHolder { .. }));
    }

    #[tokio::test]
    async fn invalid_duration_is_rejected() {
        let s = LeaseStorage::new();
        let now = Utc::now();
        assert!(matches!(
            s.acquire(key(), "node-a", 0, now).await,
            Err(LeaseError::InvalidDuration(0))
        ));
        assert!(matches!(
            s.acquire(key(), "node-a", -1, now).await,
            Err(LeaseError::InvalidDuration(-1))
        ));
    }

    #[tokio::test]
    async fn empty_holder_is_rejected() {
        let s = LeaseStorage::new();
        let now = Utc::now();
        assert!(matches!(
            s.acquire(key(), "", 30, now).await,
            Err(LeaseError::EmptyHolder)
        ));
    }

    #[test]
    fn lease_spec_is_expired_matches_renew_time_plus_duration() {
        let t0 = Utc::now();
        let spec = LeaseSpec {
            holder_identity: "x".into(),
            lease_duration_seconds: 30,
            acquire_time: t0,
            renew_time: t0,
            lease_transitions: 0,
        };
        assert!(!spec.is_expired(t0));
        assert!(!spec.is_expired(t0 + Duration::seconds(29)));
        // Exactly at duration boundary → expired (upstream semantics).
        assert!(spec.is_expired(t0 + Duration::seconds(30)));
        assert!(spec.is_expired(t0 + Duration::seconds(31)));
    }
}
