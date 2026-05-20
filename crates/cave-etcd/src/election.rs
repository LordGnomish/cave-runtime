// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `v3election` RPC service.
//!
//! Upstream: `server/etcdserver/api/v3election/v3election.go`.
//!
//! The election service mirrors etcd's gRPC `Election` surface
//! (`Campaign`, `Proclaim`, `Leader`, `Resign`, `Observe`). The
//! underlying state machine is the existing `DistElection` primitive
//! in [`crate::concurrency`] — this module is the top-level RPC
//! façade that exposes it as a *named* election keyed by an arbitrary
//! UTF-8 election name (the upstream `name` byte string), with one
//! `DistElection` per name.
//!
//! Unlike etcd's gRPC streaming `Observe`, we expose `observe` as a
//! point-in-time snapshot (Vec of leader history) plus an async
//! `subscribe` channel for live leader changes. This keeps the model
//! ergonomic in Rust while preserving the observable contract.
//!
//! Each election is keyed by its `name`. Internally we store the
//! ordered queue of `(create_revision, lease_id, value)` rows; the
//! smallest `create_revision` is the leader.

use crate::concurrency::{DistElection, ElectionCandidate};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;

/// Leader response returned by `campaign` / `leader`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderKey {
    /// The election name (upstream `name`).
    pub name: Vec<u8>,
    /// Stringified key the leader claims (upstream key in mod_revision form).
    pub key: Vec<u8>,
    /// Revision at which this leader registered (upstream `create_revision`).
    pub create_revision: u64,
    /// Leader's lease id.
    pub lease: i64,
}

/// One observed leader event published to `subscribe` receivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderEvent {
    pub name: Vec<u8>,
    pub leader: Option<LeaderKey>,
    pub value: Vec<u8>,
}

/// Errors returned by the election service.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ElectionError {
    #[error("election has no leader")]
    NoLeader,
    #[error("not the current leader")]
    NotLeader,
    #[error("election name must not be empty")]
    EmptyName,
    #[error("lease id must be positive (got {0})")]
    InvalidLease(i64),
}

/// The election RPC service.
///
/// Maintains a map of named elections (one `DistElection` per name)
/// plus a global revision counter so leader keys can be reconstructed
/// in the upstream `<name>/<create_revision>` shape.
pub struct ElectionService {
    elections: RwLock<HashMap<Vec<u8>, Arc<DistElection>>>,
    subscribers: Mutex<HashMap<Vec<u8>, broadcast::Sender<LeaderEvent>>>,
    global_rev: AtomicU64,
}

impl Default for ElectionService {
    fn default() -> Self {
        Self::new()
    }
}

impl ElectionService {
    pub fn new() -> Self {
        Self {
            elections: RwLock::new(HashMap::new()),
            subscribers: Mutex::new(HashMap::new()),
            global_rev: AtomicU64::new(0),
        }
    }

    /// Bump the global revision counter (mirrors mvcc revision bumps
    /// triggered by election state changes).
    fn bump_rev(&self) -> u64 {
        self.global_rev.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Look up (or lazily create) the per-name election.
    fn get_or_create(&self, name: &[u8]) -> Arc<DistElection> {
        if let Some(e) = self.elections.read().unwrap().get(name) {
            return e.clone();
        }
        let mut w = self.elections.write().unwrap();
        w.entry(name.to_vec())
            .or_insert_with(|| {
                let prefix = format!("/election/{}", String::from_utf8_lossy(name));
                Arc::new(DistElection::new(prefix))
            })
            .clone()
    }

    /// Build a leader key for the given election + candidate row.
    fn leader_key(name: &[u8], c: &ElectionCandidate) -> LeaderKey {
        let key = format!(
            "/election/{}/{:016x}",
            String::from_utf8_lossy(name),
            c.create_revision
        )
        .into_bytes();
        LeaderKey {
            name: name.to_vec(),
            key,
            create_revision: c.create_revision,
            lease: c.lease_id,
        }
    }

    /// Notify subscribers of a leader change.
    fn publish(&self, name: &[u8], leader: Option<LeaderKey>, value: Vec<u8>) {
        let subs = self.subscribers.lock().unwrap();
        if let Some(tx) = subs.get(name) {
            let _ = tx.send(LeaderEvent {
                name: name.to_vec(),
                leader,
                value,
            });
        }
    }

    /// Campaign for leadership. Blocks-style: returns `(LeaderKey,
    /// became_leader)`. Caller can poll `leader(name)` until they win.
    pub fn campaign(
        &self,
        name: &[u8],
        lease: i64,
        value: Vec<u8>,
    ) -> Result<(LeaderKey, bool), ElectionError> {
        if name.is_empty() {
            return Err(ElectionError::EmptyName);
        }
        if lease <= 0 {
            return Err(ElectionError::InvalidLease(lease));
        }
        let election = self.get_or_create(name);
        let (rev, became) = election.campaign(lease, value.clone());
        let _ = self.bump_rev();
        let cand = election
            .observe()
            .into_iter()
            .find(|c| c.create_revision == rev)
            .expect("just-campaigned candidate exists");
        let key = Self::leader_key(name, &cand);
        if became {
            self.publish(name, Some(key.clone()), value);
        }
        Ok((key, became))
    }

    /// Update the leader's published value without losing leadership.
    pub fn proclaim(
        &self,
        name: &[u8],
        lease: i64,
        value: Vec<u8>,
    ) -> Result<(), ElectionError> {
        let election = self
            .elections
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or(ElectionError::NoLeader)?;
        election
            .proclaim(lease, value.clone())
            .map_err(|e| match e {
                crate::concurrency::ConcurrencyError::NoLeader => ElectionError::NoLeader,
                crate::concurrency::ConcurrencyError::NotLeader => ElectionError::NotLeader,
                _ => ElectionError::NotLeader,
            })?;
        let _ = self.bump_rev();
        if let Some(leader) = election.leader() {
            let key = Self::leader_key(name, &leader);
            self.publish(name, Some(key), value);
        }
        Ok(())
    }

    /// Return the current leader for the named election.
    pub fn leader(&self, name: &[u8]) -> Result<(LeaderKey, Vec<u8>), ElectionError> {
        let election = self
            .elections
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or(ElectionError::NoLeader)?;
        let cand = election.leader().ok_or(ElectionError::NoLeader)?;
        let key = Self::leader_key(name, &cand);
        Ok((key, cand.value))
    }

    /// Voluntarily resign — the next candidate (smallest create_revision)
    /// is promoted. Returns the new leader's lease id if any.
    pub fn resign(&self, name: &[u8], lease: i64) -> Result<Option<i64>, ElectionError> {
        let election = self
            .elections
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or(ElectionError::NoLeader)?;
        let new_leader_lease = election.resign(lease).map_err(|e| match e {
            crate::concurrency::ConcurrencyError::NoLeader => ElectionError::NoLeader,
            crate::concurrency::ConcurrencyError::NotLeader => ElectionError::NotLeader,
            _ => ElectionError::NotLeader,
        })?;
        let _ = self.bump_rev();
        let new_event_leader = election.leader();
        let value = new_event_leader
            .as_ref()
            .map(|c| c.value.clone())
            .unwrap_or_default();
        let new_key = new_event_leader.map(|c| Self::leader_key(name, &c));
        self.publish(name, new_key, value);
        Ok(new_leader_lease)
    }

    /// Drop a lease (e.g. expired by lessor) — returns the new leader's
    /// lease id if leadership transferred.
    pub fn expire_lease(&self, name: &[u8], lease: i64) -> Option<i64> {
        let election = self.elections.read().unwrap().get(name).cloned()?;
        let res = election.expire_lease(lease);
        let _ = self.bump_rev();
        let new_event_leader = election.leader();
        let value = new_event_leader
            .as_ref()
            .map(|c| c.value.clone())
            .unwrap_or_default();
        let new_key = new_event_leader.map(|c| Self::leader_key(name, &c));
        self.publish(name, new_key, value);
        res
    }

    /// Snapshot of all candidates in queue order (smallest
    /// create_revision first). Mirrors a point-in-time `Observe`.
    pub fn observe(&self, name: &[u8]) -> Vec<LeaderKey> {
        let Some(election) = self.elections.read().unwrap().get(name).cloned() else {
            return Vec::new();
        };
        election
            .observe()
            .iter()
            .map(|c| Self::leader_key(name, c))
            .collect()
    }

    /// Subscribe to leader-change events for the named election. The
    /// returned receiver yields a `LeaderEvent` each time campaign /
    /// proclaim / resign / expire mutates leadership.
    pub fn subscribe(&self, name: &[u8]) -> broadcast::Receiver<LeaderEvent> {
        let mut subs = self.subscribers.lock().unwrap();
        let tx = subs
            .entry(name.to_vec())
            .or_insert_with(|| broadcast::channel::<LeaderEvent>(64).0);
        tx.subscribe()
    }

    /// Global revision (monotonically increasing across all elections).
    pub fn revision(&self) -> u64 {
        self.global_rev.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campaign_first_candidate_becomes_leader() {
        let svc = ElectionService::new();
        let (key, became) = svc
            .campaign(b"my-election", 1001, b"alice".to_vec())
            .expect("campaign ok");
        assert!(became);
        assert_eq!(key.lease, 1001);
        assert_eq!(key.name, b"my-election");
        let (leader, value) = svc.leader(b"my-election").expect("has leader");
        assert_eq!(leader.lease, 1001);
        assert_eq!(value, b"alice");
    }

    #[test]
    fn campaign_second_candidate_is_queued_not_leader() {
        let svc = ElectionService::new();
        let (_, became1) = svc
            .campaign(b"e", 1, b"first".to_vec())
            .expect("c1");
        let (_, became2) = svc
            .campaign(b"e", 2, b"second".to_vec())
            .expect("c2");
        assert!(became1);
        assert!(!became2);
        let observed = svc.observe(b"e");
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0].lease, 1);
        assert_eq!(observed[1].lease, 2);
    }

    #[test]
    fn proclaim_updates_leader_value() {
        let svc = ElectionService::new();
        svc.campaign(b"e", 1, b"v1".to_vec()).unwrap();
        svc.proclaim(b"e", 1, b"v2".to_vec()).expect("proclaim ok");
        let (_, val) = svc.leader(b"e").unwrap();
        assert_eq!(val, b"v2");
    }

    #[test]
    fn proclaim_from_non_leader_errors() {
        let svc = ElectionService::new();
        svc.campaign(b"e", 1, b"v1".to_vec()).unwrap();
        svc.campaign(b"e", 2, b"v2".to_vec()).unwrap();
        let err = svc.proclaim(b"e", 2, b"hijack".to_vec()).unwrap_err();
        assert_eq!(err, ElectionError::NotLeader);
    }

    #[test]
    fn resign_promotes_next_candidate() {
        let svc = ElectionService::new();
        svc.campaign(b"e", 1, b"first".to_vec()).unwrap();
        svc.campaign(b"e", 2, b"second".to_vec()).unwrap();
        let new_leader = svc.resign(b"e", 1).expect("resign ok");
        assert_eq!(new_leader, Some(2));
        let (leader, _) = svc.leader(b"e").unwrap();
        assert_eq!(leader.lease, 2);
    }

    #[test]
    fn observe_returns_all_candidates_in_order() {
        let svc = ElectionService::new();
        svc.campaign(b"e", 10, b"a".to_vec()).unwrap();
        svc.campaign(b"e", 20, b"b".to_vec()).unwrap();
        svc.campaign(b"e", 30, b"c".to_vec()).unwrap();
        let obs = svc.observe(b"e");
        assert_eq!(obs.len(), 3);
        assert_eq!(obs[0].lease, 10);
        assert_eq!(obs[2].lease, 30);
    }

    #[test]
    fn leader_on_empty_election_errors() {
        let svc = ElectionService::new();
        let err = svc.leader(b"never-campaigned").unwrap_err();
        assert_eq!(err, ElectionError::NoLeader);
    }

    #[test]
    fn empty_name_rejected() {
        let svc = ElectionService::new();
        let err = svc.campaign(b"", 1, b"v".to_vec()).unwrap_err();
        assert_eq!(err, ElectionError::EmptyName);
    }

    #[test]
    fn nonpositive_lease_rejected() {
        let svc = ElectionService::new();
        let err = svc.campaign(b"e", 0, b"v".to_vec()).unwrap_err();
        assert_eq!(err, ElectionError::InvalidLease(0));
        let err = svc.campaign(b"e", -5, b"v".to_vec()).unwrap_err();
        assert_eq!(err, ElectionError::InvalidLease(-5));
    }

    #[test]
    fn expire_lease_promotes_next() {
        let svc = ElectionService::new();
        svc.campaign(b"e", 1, b"a".to_vec()).unwrap();
        svc.campaign(b"e", 2, b"b".to_vec()).unwrap();
        let new_leader = svc.expire_lease(b"e", 1);
        assert_eq!(new_leader, Some(2));
        let (leader, _) = svc.leader(b"e").unwrap();
        assert_eq!(leader.lease, 2);
    }

    #[test]
    fn revision_increases_per_op() {
        let svc = ElectionService::new();
        let r0 = svc.revision();
        svc.campaign(b"e", 1, b"v".to_vec()).unwrap();
        let r1 = svc.revision();
        svc.proclaim(b"e", 1, b"v2".to_vec()).unwrap();
        let r2 = svc.revision();
        svc.resign(b"e", 1).unwrap();
        let r3 = svc.revision();
        assert!(r1 > r0);
        assert!(r2 > r1);
        assert!(r3 > r2);
    }

    #[test]
    fn separate_names_have_separate_leaders() {
        let svc = ElectionService::new();
        svc.campaign(b"alpha", 1, b"a".to_vec()).unwrap();
        svc.campaign(b"beta", 2, b"b".to_vec()).unwrap();
        let (la, _) = svc.leader(b"alpha").unwrap();
        let (lb, _) = svc.leader(b"beta").unwrap();
        assert_eq!(la.lease, 1);
        assert_eq!(lb.lease, 2);
    }

    #[tokio::test]
    async fn subscribe_emits_event_on_campaign_win() {
        let svc = ElectionService::new();
        let mut rx = svc.subscribe(b"e");
        svc.campaign(b"e", 1, b"v".to_vec()).unwrap();
        let ev = rx.recv().await.expect("got event");
        assert_eq!(ev.name, b"e");
        assert_eq!(ev.leader.as_ref().unwrap().lease, 1);
        assert_eq!(ev.value, b"v");
    }

    #[tokio::test]
    async fn subscribe_emits_on_resign_with_new_leader() {
        let svc = ElectionService::new();
        svc.campaign(b"e", 1, b"a".to_vec()).unwrap();
        svc.campaign(b"e", 2, b"b".to_vec()).unwrap();
        let mut rx = svc.subscribe(b"e");
        svc.resign(b"e", 1).unwrap();
        let ev = rx.recv().await.expect("event");
        assert_eq!(ev.leader.as_ref().unwrap().lease, 2);
    }

    #[test]
    fn leader_key_contains_name_and_revision_hex() {
        let svc = ElectionService::new();
        let (key, _) = svc.campaign(b"my-elect", 7, b"v".to_vec()).unwrap();
        let s = String::from_utf8_lossy(&key.key).to_string();
        assert!(s.starts_with("/election/my-elect/"), "{}", s);
        // Hex-suffix is 16 chars long.
        assert_eq!(s.rsplit('/').next().unwrap().len(), 16);
    }
}
