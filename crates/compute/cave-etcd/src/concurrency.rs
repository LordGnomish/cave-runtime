// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Distributed concurrency primitives — `Mutex`, `RWMutex`, and `Election`.
//!
//! Mirrors etcd's `clientv3/concurrency` package which builds these on top
//! of leases + transactions:
//!
//!   * `Mutex` — exclusive lock keyed by `<prefix>/<lease-id>`.  The holder
//!     is the one with the smallest `create_revision`; everyone else waits
//!     on the predecessor's deletion.
//!   * `RWMutex` — read/write lock built from two prefixes (`/r`, `/w`).
//!     Writers wait for both prefixes; readers only wait for the writer
//!     prefix.
//!   * `Election` — leader election keyed by `<prefix>`.  The leader is the
//!     single member with the smallest `create_revision`; resign deletes
//!     the key and a new leader emerges.  `proclaim` updates the leader
//!     value without re-electing.
//!
//! Mirrors etcd v3.6.10
//!   `client/v3/concurrency/mutex.go`,
//!   `client/v3/concurrency/rwmutex.go`,
//!   `client/v3/concurrency/election.go`.
//!
//! The implementation is in-memory and lease-aware — it composes with
//! [`crate::store::KvStore`] via a small adapter so locks survive lease
//! revocation, but the queueing logic itself does not require an
//! out-of-process broker.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

// ── Errors ────────────────────────────────────────────────────────────────

/// Errors specific to the concurrency primitives.
#[derive(Debug, PartialEq, Eq)]
pub enum ConcurrencyError {
    /// `unlock`/`resign` called on a key the caller doesn't own.
    NotOwner,
    /// Lock attempt would have blocked but the caller asked for `try_lock`.
    WouldBlock,
    /// The supplied lease id is not registered.
    LeaseNotFound(i64),
    /// `proclaim` called by a non-leader.
    NotLeader,
    /// No leader currently elected.
    NoLeader,
}

impl std::fmt::Display for ConcurrencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotOwner => write!(f, "caller does not own the lock"),
            Self::WouldBlock => write!(f, "lock would block"),
            Self::LeaseNotFound(id) => write!(f, "lease not found: {id}"),
            Self::NotLeader => write!(f, "caller is not the leader"),
            Self::NoLeader => write!(f, "no leader currently elected"),
        }
    }
}

impl std::error::Error for ConcurrencyError {}

// ── Mutex ────────────────────────────────────────────────────────────────

/// One queued waiter on a [`Mutex`].
#[derive(Debug, Clone)]
pub struct MutexEntry {
    /// Caller-supplied lease id.  Releasing the lease releases the lock.
    pub lease_id: i64,
    /// Revision the entry was inserted at — used by the "smallest rev wins"
    /// rule.
    pub create_revision: u64,
}

/// In-memory distributed mutex modelled on etcd's `clientv3/concurrency.Mutex`.
///
/// Acquisition order is **strictly** by `create_revision` so a slow caller
/// that registered first never gets jumped.  This matches the `pfx/lease_id`
/// + `WithFirstCreate` semantics of the upstream implementation.
pub struct DistMutex {
    /// User-visible key prefix — appears in audit logs and tests.
    pub prefix: String,
    revision: AtomicU64,
    inner: Mutex<MutexInner>,
}

#[derive(Debug, Default)]
struct MutexInner {
    /// `(create_revision, lease_id)` ordered map.  First entry is the holder.
    queue: BTreeMap<u64, MutexEntry>,
}

impl DistMutex {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            revision: AtomicU64::new(0),
            inner: Mutex::new(MutexInner::default()),
        }
    }

    fn next_rev(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Lock — register the lease and become the holder once `create_revision`
    /// is the smallest in the queue.  Returns the create_revision the caller
    /// was assigned.  If `WouldBlock` is returned, the caller is *queued*
    /// and must retry until [`is_owner`] returns true.
    ///
    /// The single-process semantics: returns `Ok(rev)` if we got the lock
    /// (smallest rev), `Err(WouldBlock)` if queued but not yet head.
    pub fn lock(&self, lease_id: i64) -> Result<u64, ConcurrencyError> {
        let rev = self.next_rev();
        let mut inner = self.inner.lock().unwrap();
        // Re-entrant-safe: if same lease already present, return its rev.
        if let Some((&existing_rev, _)) = inner.queue.iter().find(|(_, e)| e.lease_id == lease_id) {
            return Ok(existing_rev);
        }
        inner.queue.insert(
            rev,
            MutexEntry {
                lease_id,
                create_revision: rev,
            },
        );
        // Holder is the entry with the smallest rev.
        let head = *inner.queue.keys().next().unwrap();
        if head == rev {
            Ok(rev)
        } else {
            Err(ConcurrencyError::WouldBlock)
        }
    }

    /// `try_lock` — succeeds only if the queue is empty.  Mirrors
    /// `clientv3/concurrency.Mutex.TryLock`.
    pub fn try_lock(&self, lease_id: i64) -> Result<u64, ConcurrencyError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.queue.is_empty() {
            return Err(ConcurrencyError::WouldBlock);
        }
        let rev = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        inner.queue.insert(
            rev,
            MutexEntry {
                lease_id,
                create_revision: rev,
            },
        );
        Ok(rev)
    }

    /// Release the lock — must be called by the current holder.  Drops the
    /// caller from the queue regardless of position (etcd's `Unlock` is
    /// `delete pfx/lease_id`).
    pub fn unlock(&self, lease_id: i64) -> Result<(), ConcurrencyError> {
        let mut inner = self.inner.lock().unwrap();
        let mut found_rev = None;
        for (rev, entry) in inner.queue.iter() {
            if entry.lease_id == lease_id {
                found_rev = Some(*rev);
                break;
            }
        }
        let rev = found_rev.ok_or(ConcurrencyError::NotOwner)?;
        inner.queue.remove(&rev);
        Ok(())
    }

    /// `expire_lease` — invoked by the lease subsystem when a lease ID has
    /// been revoked or expired.  Removes any queued entry for that lease.
    pub fn expire_lease(&self, lease_id: i64) {
        let mut inner = self.inner.lock().unwrap();
        let revs: Vec<u64> = inner
            .queue
            .iter()
            .filter(|(_, e)| e.lease_id == lease_id)
            .map(|(r, _)| *r)
            .collect();
        for r in revs {
            inner.queue.remove(&r);
        }
    }

    /// Whether this lease currently holds the lock (smallest rev).
    pub fn is_owner(&self, lease_id: i64) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.queue.iter().next().map(|(_, e)| e.lease_id) == Some(lease_id)
    }

    /// Number of queued waiters (including the holder if any).
    pub fn queue_len(&self) -> usize {
        self.inner.lock().unwrap().queue.len()
    }

    /// Snapshot the queue — primarily for tests/debug.
    pub fn snapshot(&self) -> Vec<MutexEntry> {
        self.inner.lock().unwrap().queue.values().cloned().collect()
    }

    /// User-visible "key" for the current holder, mirroring etcd's
    /// `<prefix>/<lease-id-hex>` format.  Returns None when the lock is
    /// free.
    pub fn key(&self) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        inner
            .queue
            .iter()
            .next()
            .map(|(_, e)| format!("{}/{:x}", self.prefix, e.lease_id))
    }
}

// ── RWMutex ──────────────────────────────────────────────────────────────

/// Read/write lock — readers share, writers exclude all.
///
/// Mirrors `clientv3/concurrency.RWMutex` which coordinates two prefixes:
/// `/r/<id>` (read-holders) and `/w/<id>` (write-holders).
pub struct DistRWMutex {
    pub prefix: String,
    inner: Mutex<RwInner>,
    rev: AtomicU64,
}

#[derive(Debug, Default)]
struct RwInner {
    readers: BTreeMap<u64, i64>, // rev → lease_id
    writers: BTreeMap<u64, i64>, // rev → lease_id
}

impl DistRWMutex {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            inner: Mutex::new(RwInner::default()),
            rev: AtomicU64::new(0),
        }
    }

    fn next_rev(&self) -> u64 {
        self.rev.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Acquire a read lock.  Succeeds when no writer is queued ahead of us.
    /// Returns the revision we registered at.
    pub fn rlock(&self, lease_id: i64) -> Result<u64, ConcurrencyError> {
        let rev = self.next_rev();
        let mut inner = self.inner.lock().unwrap();
        // Block if a writer with smaller rev is already queued.
        if let Some(&w_rev) = inner.writers.keys().next() {
            if w_rev < rev {
                // Insert into readers regardless so we get fairness, but
                // signal the caller to retry.  (etcd matches this with
                // a watch on the writer key.)
                inner.readers.insert(rev, lease_id);
                return Err(ConcurrencyError::WouldBlock);
            }
        }
        inner.readers.insert(rev, lease_id);
        Ok(rev)
    }

    /// Acquire a write lock — exclusive against readers and other writers.
    /// Returns Ok if first in queue, WouldBlock otherwise.
    pub fn wlock(&self, lease_id: i64) -> Result<u64, ConcurrencyError> {
        let rev = self.next_rev();
        let mut inner = self.inner.lock().unwrap();
        let blocked = !inner.readers.is_empty() || inner.writers.keys().any(|&w| w < rev);
        inner.writers.insert(rev, lease_id);
        if blocked {
            Err(ConcurrencyError::WouldBlock)
        } else {
            Ok(rev)
        }
    }

    /// Release a previously-held read lock.
    pub fn runlock(&self, lease_id: i64) -> Result<(), ConcurrencyError> {
        let mut inner = self.inner.lock().unwrap();
        let rev = inner
            .readers
            .iter()
            .find(|(_, &l)| l == lease_id)
            .map(|(r, _)| *r);
        let rev = rev.ok_or(ConcurrencyError::NotOwner)?;
        inner.readers.remove(&rev);
        Ok(())
    }

    /// Release a previously-held write lock.
    pub fn wunlock(&self, lease_id: i64) -> Result<(), ConcurrencyError> {
        let mut inner = self.inner.lock().unwrap();
        let rev = inner
            .writers
            .iter()
            .find(|(_, &l)| l == lease_id)
            .map(|(r, _)| *r);
        let rev = rev.ok_or(ConcurrencyError::NotOwner)?;
        inner.writers.remove(&rev);
        Ok(())
    }

    pub fn reader_count(&self) -> usize {
        self.inner.lock().unwrap().readers.len()
    }
    pub fn writer_count(&self) -> usize {
        self.inner.lock().unwrap().writers.len()
    }
    pub fn has_writer(&self) -> bool {
        self.writer_count() > 0
    }

    pub fn expire_lease(&self, lease_id: i64) {
        let mut inner = self.inner.lock().unwrap();
        let r_revs: Vec<u64> = inner
            .readers
            .iter()
            .filter(|(_, &l)| l == lease_id)
            .map(|(r, _)| *r)
            .collect();
        for r in r_revs {
            inner.readers.remove(&r);
        }
        let w_revs: Vec<u64> = inner
            .writers
            .iter()
            .filter(|(_, &l)| l == lease_id)
            .map(|(r, _)| *r)
            .collect();
        for r in w_revs {
            inner.writers.remove(&r);
        }
    }
}

// ── Election ─────────────────────────────────────────────────────────────

/// One queued candidate in an [`Election`].
#[derive(Debug, Clone)]
pub struct ElectionCandidate {
    pub lease_id: i64,
    pub value: Vec<u8>,
    pub create_revision: u64,
}

/// Leader election.  Mirrors `clientv3/concurrency.Election`.
///
/// Only the candidate with the smallest `create_revision` is the leader.
/// `campaign` registers; `proclaim` updates the leader's value; `resign`
/// removes the leader and lets the next candidate become leader.
pub struct DistElection {
    pub prefix: String,
    rev: AtomicU64,
    inner: Mutex<ElectionInner>,
}

#[derive(Debug, Default)]
struct ElectionInner {
    queue: BTreeMap<u64, ElectionCandidate>,
}

impl DistElection {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            rev: AtomicU64::new(0),
            inner: Mutex::new(ElectionInner::default()),
        }
    }

    fn next_rev(&self) -> u64 {
        self.rev.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Campaign for leadership.  If queue is empty we become leader; else
    /// we're queued.  The return value is `(create_revision, became_leader)`.
    pub fn campaign(&self, lease_id: i64, value: Vec<u8>) -> (u64, bool) {
        let rev = self.next_rev();
        let mut inner = self.inner.lock().unwrap();
        let was_empty = inner.queue.is_empty();
        inner.queue.insert(
            rev,
            ElectionCandidate {
                lease_id,
                value,
                create_revision: rev,
            },
        );
        (rev, was_empty)
    }

    /// Update the leader's published value without re-electing.
    pub fn proclaim(&self, lease_id: i64, value: Vec<u8>) -> Result<(), ConcurrencyError> {
        let mut inner = self.inner.lock().unwrap();
        let leader = inner
            .queue
            .iter()
            .next()
            .map(|(r, _)| *r)
            .ok_or(ConcurrencyError::NoLeader)?;
        let entry = inner.queue.get_mut(&leader).unwrap();
        if entry.lease_id != lease_id {
            return Err(ConcurrencyError::NotLeader);
        }
        entry.value = value;
        Ok(())
    }

    /// Step down — leader removes itself.  Next-smallest candidate becomes
    /// the new leader automatically.  Returns the new leader's lease (if any).
    pub fn resign(&self, lease_id: i64) -> Result<Option<i64>, ConcurrencyError> {
        let mut inner = self.inner.lock().unwrap();
        let leader_rev = inner
            .queue
            .iter()
            .next()
            .map(|(r, _)| *r)
            .ok_or(ConcurrencyError::NoLeader)?;
        let leader_lease = inner.queue.get(&leader_rev).map(|c| c.lease_id);
        if leader_lease != Some(lease_id) {
            return Err(ConcurrencyError::NotLeader);
        }
        inner.queue.remove(&leader_rev);
        Ok(inner.queue.iter().next().map(|(_, c)| c.lease_id))
    }

    /// Currently-elected leader (smallest rev).
    pub fn leader(&self) -> Option<ElectionCandidate> {
        let inner = self.inner.lock().unwrap();
        inner.queue.iter().next().map(|(_, c)| c.clone())
    }

    /// Whether this lease holds the leadership.
    pub fn is_leader(&self, lease_id: i64) -> bool {
        self.leader().map(|c| c.lease_id) == Some(lease_id)
    }

    /// Drop a lease's candidate (e.g. when its lease expires).  Returns the
    /// new leader's lease if leadership changed.
    pub fn expire_lease(&self, lease_id: i64) -> Option<i64> {
        let mut inner = self.inner.lock().unwrap();
        let prev_leader_rev = inner.queue.iter().next().map(|(r, _)| *r);
        let prev_leader_lease =
            prev_leader_rev.and_then(|r| inner.queue.get(&r).map(|c| c.lease_id));
        let revs: Vec<u64> = inner
            .queue
            .iter()
            .filter(|(_, c)| c.lease_id == lease_id)
            .map(|(r, _)| *r)
            .collect();
        for r in revs {
            inner.queue.remove(&r);
        }
        let new_leader_lease = inner.queue.iter().next().map(|(_, c)| c.lease_id);
        if new_leader_lease != prev_leader_lease {
            new_leader_lease
        } else {
            None
        }
    }

    /// All currently-queued candidates, ordered by create_revision.
    pub fn observe(&self) -> Vec<ElectionCandidate> {
        self.inner.lock().unwrap().queue.values().cloned().collect()
    }

    pub fn queue_len(&self) -> usize {
        self.inner.lock().unwrap().queue.len()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Concurrency-package tests — feat/cave-etcd-100-pct-sprint
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Mutex ─────────────────────────────────────────────────────────

    #[test]
    fn test_mutex_first_caller_acquires() {
        // cite: clientv3/concurrency.Mutex.Lock (smallest rev wins)
        let m = DistMutex::new("/locks/foo");
        let rev = m.lock(100).unwrap();
        assert!(m.is_owner(100));
        assert!(rev >= 1);
    }

    #[test]
    fn test_mutex_second_caller_blocks() {
        // cite: clientv3/concurrency.Mutex.Lock (waiters queue behind holder)
        let m = DistMutex::new("/locks/foo");
        m.lock(100).unwrap();
        assert_eq!(m.lock(200).unwrap_err(), ConcurrencyError::WouldBlock);
        assert!(m.is_owner(100));
        assert!(!m.is_owner(200));
    }

    #[test]
    fn test_mutex_unlock_promotes_next() {
        // cite: clientv3/concurrency.Mutex.Unlock (delete pfx/lease_id)
        let m = DistMutex::new("/locks/foo");
        m.lock(100).unwrap();
        let _ = m.lock(200);
        m.unlock(100).unwrap();
        assert!(m.is_owner(200));
    }

    #[test]
    fn test_mutex_unlock_non_owner_errors() {
        // cite: clientv3/concurrency.Mutex.Unlock (caller must hold the lock)
        let m = DistMutex::new("/locks/foo");
        m.lock(100).unwrap();
        assert_eq!(m.unlock(999).unwrap_err(), ConcurrencyError::NotOwner);
    }

    #[test]
    fn test_mutex_lock_is_idempotent_for_same_lease() {
        // cite: clientv3/concurrency.Mutex.Lock — re-entrant under same lease
        let m = DistMutex::new("/locks/foo");
        let r1 = m.lock(100).unwrap();
        let r2 = m.lock(100).unwrap();
        assert_eq!(r1, r2);
        assert_eq!(m.queue_len(), 1);
    }

    #[test]
    fn test_mutex_try_lock_when_free() {
        // cite: clientv3/concurrency.Mutex.TryLock (succeeds when empty)
        let m = DistMutex::new("/locks/x");
        assert!(m.try_lock(100).is_ok());
    }

    #[test]
    fn test_mutex_try_lock_when_held() {
        // cite: clientv3/concurrency.Mutex.TryLock (fails when held)
        let m = DistMutex::new("/locks/x");
        m.lock(100).unwrap();
        assert_eq!(m.try_lock(200).unwrap_err(), ConcurrencyError::WouldBlock);
    }

    #[test]
    fn test_mutex_lease_expiry_releases() {
        // cite: clientv3/concurrency.Mutex (lease revoke ⇒ delete pfx/lease_id)
        let m = DistMutex::new("/locks/x");
        m.lock(100).unwrap();
        let _ = m.lock(200);
        m.expire_lease(100);
        assert!(m.is_owner(200));
    }

    #[test]
    fn test_mutex_fairness_order() {
        // cite: clientv3/concurrency.Mutex (smallest create_revision wins)
        let m = DistMutex::new("/locks/x");
        m.lock(1).unwrap();
        let _ = m.lock(2);
        let _ = m.lock(3);
        m.unlock(1).unwrap();
        assert!(m.is_owner(2));
        m.unlock(2).unwrap();
        assert!(m.is_owner(3));
    }

    #[test]
    fn test_mutex_key_format() {
        // cite: clientv3/concurrency.Mutex.Key() == <prefix>/<lease-id-hex>
        let m = DistMutex::new("/locks/foo");
        m.lock(0xABC).unwrap();
        assert_eq!(m.key().unwrap(), "/locks/foo/abc");
    }

    #[test]
    fn test_mutex_key_none_when_free() {
        // cite: clientv3/concurrency.Mutex.Key() (empty before campaign)
        let m = DistMutex::new("/locks/foo");
        assert!(m.key().is_none());
    }

    #[test]
    fn test_mutex_snapshot_in_order() {
        // cite: clientv3/concurrency.Mutex (queue ordered by rev)
        let m = DistMutex::new("/locks/x");
        m.lock(1).unwrap();
        let _ = m.lock(2);
        let _ = m.lock(3);
        let snap = m.snapshot();
        assert_eq!(snap.len(), 3);
        assert_eq!(snap[0].lease_id, 1);
        assert_eq!(snap[1].lease_id, 2);
        assert_eq!(snap[2].lease_id, 3);
    }

    // ── RWMutex ───────────────────────────────────────────────────────

    #[test]
    fn test_rwmutex_multiple_readers_share() {
        // cite: clientv3/concurrency.RWMutex.RLock (readers share)
        let rw = DistRWMutex::new("/rw/foo");
        rw.rlock(1).unwrap();
        rw.rlock(2).unwrap();
        rw.rlock(3).unwrap();
        assert_eq!(rw.reader_count(), 3);
    }

    #[test]
    fn test_rwmutex_writer_blocked_by_readers() {
        // cite: clientv3/concurrency.RWMutex.Lock (writer waits for readers)
        let rw = DistRWMutex::new("/rw/foo");
        rw.rlock(1).unwrap();
        assert_eq!(rw.wlock(99).unwrap_err(), ConcurrencyError::WouldBlock);
    }

    #[test]
    fn test_rwmutex_readers_blocked_by_earlier_writer() {
        // cite: clientv3/concurrency.RWMutex.RLock (writer-first fairness)
        let rw = DistRWMutex::new("/rw/foo");
        let _ = rw.wlock(99);
        assert_eq!(rw.rlock(1).unwrap_err(), ConcurrencyError::WouldBlock);
    }

    #[test]
    fn test_rwmutex_runlock_releases() {
        // cite: clientv3/concurrency.RWMutex.RUnlock
        let rw = DistRWMutex::new("/rw/foo");
        rw.rlock(1).unwrap();
        rw.rlock(2).unwrap();
        rw.runlock(1).unwrap();
        assert_eq!(rw.reader_count(), 1);
    }

    #[test]
    fn test_rwmutex_wunlock_lets_readers_in() {
        // cite: clientv3/concurrency.RWMutex.Unlock
        let rw = DistRWMutex::new("/rw/foo");
        rw.wlock(99).unwrap();
        rw.wunlock(99).unwrap();
        rw.rlock(1).unwrap();
        assert_eq!(rw.reader_count(), 1);
    }

    #[test]
    fn test_rwmutex_runlock_non_owner() {
        // cite: clientv3/concurrency.RWMutex (caller must hold the lock)
        let rw = DistRWMutex::new("/rw/foo");
        assert_eq!(rw.runlock(1).unwrap_err(), ConcurrencyError::NotOwner);
    }

    #[test]
    fn test_rwmutex_wunlock_non_owner() {
        // cite: clientv3/concurrency.RWMutex (caller must hold the lock)
        let rw = DistRWMutex::new("/rw/foo");
        rw.wlock(99).unwrap();
        assert_eq!(rw.wunlock(1).unwrap_err(), ConcurrencyError::NotOwner);
    }

    #[test]
    fn test_rwmutex_lease_expiry_releases_reader() {
        // cite: lease revoke removes pfx/r/lease_id
        let rw = DistRWMutex::new("/rw/foo");
        rw.rlock(1).unwrap();
        rw.rlock(2).unwrap();
        rw.expire_lease(1);
        assert_eq!(rw.reader_count(), 1);
    }

    #[test]
    fn test_rwmutex_lease_expiry_releases_writer() {
        // cite: lease revoke removes pfx/w/lease_id
        let rw = DistRWMutex::new("/rw/foo");
        rw.wlock(1).unwrap();
        rw.expire_lease(1);
        assert!(!rw.has_writer());
    }

    // ── Election ──────────────────────────────────────────────────────

    #[test]
    fn test_election_first_campaign_wins() {
        // cite: clientv3/concurrency.Election.Campaign (smallest rev wins)
        let e = DistElection::new("/elections/leader");
        let (_rev, became) = e.campaign(100, b"node-A".to_vec());
        assert!(became);
        assert!(e.is_leader(100));
    }

    #[test]
    fn test_election_second_campaign_loses() {
        // cite: clientv3/concurrency.Election.Campaign (queued behind leader)
        let e = DistElection::new("/elections/leader");
        e.campaign(100, b"A".to_vec());
        let (_rev, became) = e.campaign(200, b"B".to_vec());
        assert!(!became);
        assert!(e.is_leader(100));
    }

    #[test]
    fn test_election_resign_promotes_next() {
        // cite: clientv3/concurrency.Election.Resign
        let e = DistElection::new("/e");
        e.campaign(1, b"A".to_vec());
        e.campaign(2, b"B".to_vec());
        let new = e.resign(1).unwrap();
        assert_eq!(new, Some(2));
        assert!(e.is_leader(2));
    }

    #[test]
    fn test_election_resign_last_returns_none() {
        // cite: clientv3/concurrency.Election.Resign (no successor)
        let e = DistElection::new("/e");
        e.campaign(1, b"A".to_vec());
        assert_eq!(e.resign(1).unwrap(), None);
        assert!(e.leader().is_none());
    }

    #[test]
    fn test_election_resign_non_leader_errors() {
        // cite: clientv3/concurrency.Election.Resign (must be the leader)
        let e = DistElection::new("/e");
        e.campaign(1, b"A".to_vec());
        e.campaign(2, b"B".to_vec());
        assert_eq!(e.resign(2).unwrap_err(), ConcurrencyError::NotLeader);
    }

    #[test]
    fn test_election_resign_with_no_leader_errors() {
        // cite: clientv3/concurrency.Election.Resign (no leader present)
        let e = DistElection::new("/e");
        assert_eq!(e.resign(1).unwrap_err(), ConcurrencyError::NoLeader);
    }

    #[test]
    fn test_election_proclaim_updates_value() {
        // cite: clientv3/concurrency.Election.Proclaim
        let e = DistElection::new("/e");
        e.campaign(1, b"A".to_vec());
        e.proclaim(1, b"A-v2".to_vec()).unwrap();
        assert_eq!(e.leader().unwrap().value, b"A-v2");
    }

    #[test]
    fn test_election_proclaim_non_leader() {
        // cite: clientv3/concurrency.Election.Proclaim (must be leader)
        let e = DistElection::new("/e");
        e.campaign(1, b"A".to_vec());
        e.campaign(2, b"B".to_vec());
        assert_eq!(
            e.proclaim(2, b"x".to_vec()).unwrap_err(),
            ConcurrencyError::NotLeader
        );
    }

    #[test]
    fn test_election_proclaim_with_no_leader() {
        // cite: clientv3/concurrency.Election.Proclaim
        let e = DistElection::new("/e");
        assert_eq!(
            e.proclaim(1, b"x".to_vec()).unwrap_err(),
            ConcurrencyError::NoLeader
        );
    }

    #[test]
    fn test_election_observe_returns_full_queue() {
        // cite: clientv3/concurrency.Election.Observe
        let e = DistElection::new("/e");
        e.campaign(1, b"A".to_vec());
        e.campaign(2, b"B".to_vec());
        e.campaign(3, b"C".to_vec());
        let candidates = e.observe();
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].lease_id, 1);
        assert_eq!(candidates[2].lease_id, 3);
    }

    #[test]
    fn test_election_lease_expiry_promotes_next() {
        // cite: clientv3/concurrency.Election (lease revoke == resign)
        let e = DistElection::new("/e");
        e.campaign(1, b"A".to_vec());
        e.campaign(2, b"B".to_vec());
        let new_leader = e.expire_lease(1);
        assert_eq!(new_leader, Some(2));
        assert!(e.is_leader(2));
    }

    #[test]
    fn test_election_lease_expiry_for_non_leader_keeps_leader() {
        // cite: clientv3/concurrency.Election (non-leader expire ⇒ no change)
        let e = DistElection::new("/e");
        e.campaign(1, b"A".to_vec());
        e.campaign(2, b"B".to_vec());
        let change = e.expire_lease(2);
        assert_eq!(change, None);
        assert!(e.is_leader(1));
    }

    #[test]
    fn test_election_value_is_carried_to_observers() {
        // cite: clientv3/concurrency.Election (leader value is what observers see)
        let e = DistElection::new("/e");
        e.campaign(1, b"node-A".to_vec());
        assert_eq!(e.leader().unwrap().value, b"node-A");
    }
}
