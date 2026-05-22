// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lock-manager — deeper-cut distributed-lock primitives layered on
//! [`crate::concurrency::DistMutex`] / [`crate::concurrency::DistElection`].
//!
//! Adds the bits the v3.6 clientv3/recipes package ships beyond the bare
//! Mutex/Election:
//!
//!   * **header capture** — every successful lock returns the
//!     `acquired_revision` so the caller can drive a Watch/Compare against
//!     it (matches `Mutex.Header()` in client/v3/concurrency),
//!   * **deadline locks** — `TryLockUntil` with an absolute deadline,
//!   * **multi-lock acquire** — order keys to prevent deadlock,
//!   * **named-lock registry** — `LockManager` keyed by string,
//!   * **leader epoch** — every Election leader gets a monotonically
//!     increasing epoch number so consumers can detect leadership change,
//!   * **observer channels** — `LeaderObserver` sees a sequence of
//!     `(epoch, lease, value)` snapshots.
//!
//! Mirrors etcd v3.6.10
//!   `client/v3/concurrency/mutex.go#Header`,
//!   `client/v3/concurrency/election.go#Observe`,
//!   `client/v3/concurrency/election.go#Resign` (epoch transition),
//!   `client/v3/recipes/double_barrier.go` (deadline / N-way semantics).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

// ── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum LockManagerError {
    /// `try_lock_until` deadline passed before acquisition.
    DeadlineExceeded,
    /// `acquire_all` saw a duplicate key.
    DuplicateKey(String),
    /// Caller does not own the lock they tried to release.
    NotOwner,
    /// No leader currently elected.
    NoLeader,
    /// Caller is not the leader.
    NotLeader,
    /// Named lock not found in manager.
    UnknownLock(String),
}

impl std::fmt::Display for LockManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeadlineExceeded => write!(f, "deadline exceeded"),
            Self::DuplicateKey(k) => write!(f, "duplicate lock key: {k}"),
            Self::NotOwner => write!(f, "caller does not own the lock"),
            Self::NoLeader => write!(f, "no leader currently elected"),
            Self::NotLeader => write!(f, "caller is not leader"),
            Self::UnknownLock(k) => write!(f, "unknown lock: {k}"),
        }
    }
}

impl std::error::Error for LockManagerError {}

// ── LockHeader — captured at acquisition ─────────────────────────────────

/// Header attached to every successful lock — mirrors the
/// `ResponseHeader` etcd's `Mutex.Lock()` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockHeader {
    pub acquired_revision: u64,
    pub lease_id: i64,
    pub key: String,
    pub acquired_at: Instant,
}

impl LockHeader {
    pub fn age(&self) -> Duration {
        self.acquired_at.elapsed()
    }
}

// ── HeaderedMutex ────────────────────────────────────────────────────────

/// Single-process mutex that records a [`LockHeader`] on each successful
/// acquisition.  Smallest-revision-wins fairness like
/// [`crate::concurrency::DistMutex`].
pub struct HeaderedMutex {
    key: String,
    revision: AtomicU64,
    state: Mutex<HeaderedInner>,
}

#[derive(Default)]
struct HeaderedInner {
    /// Ordered by acquisition rev; first entry is the holder.
    queue: BTreeMap<u64, i64>, // rev -> lease_id
    holder_header: Option<LockHeader>,
}

impl HeaderedMutex {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            revision: AtomicU64::new(0),
            state: Mutex::new(HeaderedInner::default()),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    fn next_rev(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Acquire — blocks (in the abstract sense) until first in queue.
    /// Returns the header if granted, or `WouldBlock` if queued.
    pub fn lock(&self, lease_id: i64) -> Result<LockHeader, super::concurrency::ConcurrencyError> {
        let rev = self.next_rev();
        let mut s = self.state.lock().unwrap();
        // Re-entrant under same lease.
        if let Some(h) = &s.holder_header {
            if h.lease_id == lease_id {
                return Ok(h.clone());
            }
        }
        s.queue.insert(rev, lease_id);
        let head = *s.queue.keys().next().unwrap();
        if head == rev {
            let header = LockHeader {
                acquired_revision: rev,
                lease_id,
                key: self.key.clone(),
                acquired_at: Instant::now(),
            };
            s.holder_header = Some(header.clone());
            Ok(header)
        } else {
            Err(super::concurrency::ConcurrencyError::WouldBlock)
        }
    }

    /// Try to acquire by `deadline`.  Loops the queue once; succeeds only
    /// if we end up at the head before the deadline.
    pub fn try_lock_until(
        &self,
        lease_id: i64,
        deadline: Instant,
    ) -> Result<LockHeader, LockManagerError> {
        // Single-process model: register, then check head.  If not head,
        // poll until deadline.
        let _ = self.lock(lease_id);
        loop {
            if Instant::now() >= deadline {
                return Err(LockManagerError::DeadlineExceeded);
            }
            let s = self.state.lock().unwrap();
            if let Some(h) = &s.holder_header {
                if h.lease_id == lease_id {
                    return Ok(h.clone());
                }
            }
            drop(s);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Release — only the current holder may unlock.
    pub fn unlock(&self, lease_id: i64) -> Result<(), LockManagerError> {
        let mut s = self.state.lock().unwrap();
        let holder_rev = s.queue.iter().next().map(|(r, _)| *r);
        let holder_lease = s.queue.iter().next().map(|(_, l)| *l);
        if holder_lease != Some(lease_id) {
            return Err(LockManagerError::NotOwner);
        }
        if let Some(rev) = holder_rev {
            s.queue.remove(&rev);
        }
        s.holder_header = None;
        // Promote next.
        if let Some((&rev, &next_lease)) = s.queue.iter().next() {
            s.holder_header = Some(LockHeader {
                acquired_revision: rev,
                lease_id: next_lease,
                key: self.key.clone(),
                acquired_at: Instant::now(),
            });
        }
        Ok(())
    }

    pub fn header(&self) -> Option<LockHeader> {
        self.state.lock().unwrap().holder_header.clone()
    }

    pub fn queue_len(&self) -> usize {
        self.state.lock().unwrap().queue.len()
    }
}

// ── Multi-lock acquire ───────────────────────────────────────────────────

/// Acquire several headered locks atomically (sorted to prevent deadlock).
/// Returns the headers in *original* request order; the function locks
/// them in *sorted* order under the hood.
pub fn acquire_all(
    locks: &HashMap<String, HeaderedMutex>,
    keys: &[String],
    lease_id: i64,
) -> Result<Vec<LockHeader>, LockManagerError> {
    // Detect duplicates.
    let mut seen = BTreeSet::new();
    for k in keys {
        if !seen.insert(k.clone()) {
            return Err(LockManagerError::DuplicateKey(k.clone()));
        }
    }
    let mut sorted: Vec<&String> = keys.iter().collect();
    sorted.sort();
    let mut acquired: Vec<(String, LockHeader)> = Vec::new();
    for k in &sorted {
        let lock = locks
            .get(k.as_str())
            .ok_or_else(|| LockManagerError::UnknownLock(k.to_string()))?;
        match lock.lock(lease_id) {
            Ok(h) => acquired.push((k.to_string(), h)),
            Err(_) => {
                // Roll back.
                for (rk, _) in acquired.iter().rev() {
                    let _ = locks.get(rk.as_str()).unwrap().unlock(lease_id);
                }
                return Err(LockManagerError::DeadlineExceeded);
            }
        }
    }
    // Re-order acquired headers to match original `keys`.
    let mut by_key: HashMap<String, LockHeader> = acquired.into_iter().collect();
    Ok(keys.iter().map(|k| by_key.remove(k).unwrap()).collect())
}

/// Release a previously-`acquire_all`'d set.
pub fn release_all(
    locks: &HashMap<String, HeaderedMutex>,
    keys: &[String],
    lease_id: i64,
) -> Result<(), LockManagerError> {
    for k in keys {
        let lock = locks
            .get(k.as_str())
            .ok_or_else(|| LockManagerError::UnknownLock(k.to_string()))?;
        lock.unlock(lease_id)?;
    }
    Ok(())
}

// ── LockManager — named registry ──────────────────────────────────────────

/// Thread-safe registry of named [`HeaderedMutex`]es.  Used by tests to
/// emulate the `client/v3/concurrency.NewMutex(prefix)` pattern.
pub struct LockManager {
    inner: RwLock<HashMap<String, HeaderedMutex>>,
}

impl LockManager {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_or_create(&self, name: impl Into<String>) -> String {
        let name = name.into();
        let mut g = self.inner.write().unwrap();
        if !g.contains_key(&name) {
            g.insert(name.clone(), HeaderedMutex::new(name.clone()));
        }
        name
    }

    pub fn lock(&self, name: &str, lease_id: i64) -> Result<LockHeader, LockManagerError> {
        let g = self.inner.read().unwrap();
        let lock = g
            .get(name)
            .ok_or_else(|| LockManagerError::UnknownLock(name.to_string()))?;
        lock.lock(lease_id).map_err(|_| LockManagerError::NotOwner)
    }

    pub fn unlock(&self, name: &str, lease_id: i64) -> Result<(), LockManagerError> {
        let g = self.inner.read().unwrap();
        let lock = g
            .get(name)
            .ok_or_else(|| LockManagerError::UnknownLock(name.to_string()))?;
        lock.unlock(lease_id)
    }

    pub fn header(&self, name: &str) -> Option<LockHeader> {
        self.inner
            .read()
            .unwrap()
            .get(name)
            .and_then(|l| l.header())
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.read().unwrap().keys().cloned().collect();
        v.sort();
        v
    }

    pub fn drop_lock(&self, name: &str) -> bool {
        self.inner.write().unwrap().remove(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for LockManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Election with epoch + observer ───────────────────────────────────────

/// One leadership epoch entry — `epoch` increases monotonically across
/// every leadership transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderEpoch {
    pub epoch: u64,
    pub lease_id: i64,
    pub value: Vec<u8>,
    pub elected_at: Instant,
}

/// Election with an explicit epoch counter and observer queue.  Built on
/// top of [`crate::concurrency::DistElection`] but exposes the new fields
/// directly so callers don't have to maintain their own counter.
pub struct EpochElection {
    state: RwLock<EpochInner>,
    epoch_counter: AtomicU64,
    /// Bounded ring of recent leadership snapshots so an observer that
    /// connects late can replay the last N transitions.
    observer_buffer_cap: usize,
    observer_buffer: Mutex<VecDeque<LeaderEpoch>>,
}

#[derive(Default)]
struct EpochInner {
    /// `(create_revision, lease_id, value)` ordered queue.
    queue: BTreeMap<u64, (i64, Vec<u8>)>,
    rev_counter: u64,
    current: Option<LeaderEpoch>,
}

impl EpochElection {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(EpochInner::default()),
            epoch_counter: AtomicU64::new(0),
            observer_buffer_cap: 32,
            observer_buffer: Mutex::new(VecDeque::new()),
        }
    }

    pub fn with_observer_cap(mut self, cap: usize) -> Self {
        self.observer_buffer_cap = cap;
        self
    }

    pub fn current_epoch(&self) -> Option<u64> {
        self.state.read().unwrap().current.as_ref().map(|c| c.epoch)
    }

    pub fn current_leader(&self) -> Option<LeaderEpoch> {
        self.state.read().unwrap().current.clone()
    }

    pub fn observer_history(&self) -> Vec<LeaderEpoch> {
        self.observer_buffer
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect()
    }

    fn record_epoch(&self, ep: LeaderEpoch) {
        let mut buf = self.observer_buffer.lock().unwrap();
        if buf.len() >= self.observer_buffer_cap {
            buf.pop_front();
        }
        buf.push_back(ep);
    }

    /// Campaign — registers in queue.  Returns `(create_revision, became_leader)`.
    pub fn campaign(&self, lease_id: i64, value: impl Into<Vec<u8>>) -> (u64, bool) {
        let mut s = self.state.write().unwrap();
        s.rev_counter += 1;
        let rev = s.rev_counter;
        let value = value.into();
        let was_empty = s.queue.is_empty();
        s.queue.insert(rev, (lease_id, value.clone()));
        if was_empty {
            let ep = LeaderEpoch {
                epoch: self.epoch_counter.fetch_add(1, Ordering::SeqCst) + 1,
                lease_id,
                value,
                elected_at: Instant::now(),
            };
            s.current = Some(ep.clone());
            drop(s);
            self.record_epoch(ep);
        }
        (rev, was_empty)
    }

    /// Resign — current leader steps down.  Returns the new leader's
    /// `(epoch, lease)` if leadership transferred.
    pub fn resign(&self, lease_id: i64) -> Result<Option<(u64, i64)>, LockManagerError> {
        let mut s = self.state.write().unwrap();
        let cur = s.current.as_ref().ok_or(LockManagerError::NoLeader)?;
        if cur.lease_id != lease_id {
            return Err(LockManagerError::NotLeader);
        }
        // Remove leader from queue.
        let leader_rev = *s.queue.iter().next().map(|(r, _)| r).unwrap();
        s.queue.remove(&leader_rev);
        s.current = None;
        // Promote next.
        if let Some((&rev, (next_lease, value))) =
            s.queue.iter().next().map(|(r, v)| (r, (v.0, v.1.clone())))
        {
            let ep = LeaderEpoch {
                epoch: self.epoch_counter.fetch_add(1, Ordering::SeqCst) + 1,
                lease_id: next_lease,
                value,
                elected_at: Instant::now(),
            };
            s.current = Some(ep.clone());
            drop(s);
            self.record_epoch(ep.clone());
            // Suppress unused-var warning while preserving rev for debug.
            let _ = rev;
            return Ok(Some((ep.epoch, ep.lease_id)));
        }
        Ok(None)
    }

    /// Update the leader's published value without re-electing.
    pub fn proclaim(
        &self,
        lease_id: i64,
        value: impl Into<Vec<u8>>,
    ) -> Result<(), LockManagerError> {
        let mut s = self.state.write().unwrap();
        let cur = s.current.as_mut().ok_or(LockManagerError::NoLeader)?;
        if cur.lease_id != lease_id {
            return Err(LockManagerError::NotLeader);
        }
        cur.value = value.into();
        Ok(())
    }

    /// Drop a candidate (lease expired).  Returns the new leader if
    /// leadership transitioned.
    pub fn expire_lease(&self, lease_id: i64) -> Option<(u64, i64)> {
        let mut s = self.state.write().unwrap();
        let prev_lease = s.current.as_ref().map(|c| c.lease_id);
        let revs: Vec<u64> = s
            .queue
            .iter()
            .filter(|(_, (l, _))| *l == lease_id)
            .map(|(r, _)| *r)
            .collect();
        for r in revs {
            s.queue.remove(&r);
        }
        // If the expired lease was the leader, promote the next.
        if prev_lease == Some(lease_id) {
            s.current = None;
            if let Some((&rev, (next_lease, value))) =
                s.queue.iter().next().map(|(r, v)| (r, (v.0, v.1.clone())))
            {
                let ep = LeaderEpoch {
                    epoch: self.epoch_counter.fetch_add(1, Ordering::SeqCst) + 1,
                    lease_id: next_lease,
                    value,
                    elected_at: Instant::now(),
                };
                s.current = Some(ep.clone());
                drop(s);
                self.record_epoch(ep.clone());
                let _ = rev;
                return Some((ep.epoch, ep.lease_id));
            }
        }
        None
    }

    pub fn queue_len(&self) -> usize {
        self.state.read().unwrap().queue.len()
    }
}

impl Default for EpochElection {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M5 deeper
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn three_locks() -> HashMap<String, HeaderedMutex> {
        let mut m = HashMap::new();
        for k in ["/a", "/b", "/c"] {
            m.insert(k.into(), HeaderedMutex::new(k));
        }
        m
    }

    // ── HeaderedMutex ──────────────────────────────────────────────────

    #[test]
    fn test_header_returned_on_lock() {
        // cite: client/v3/concurrency/mutex.go Mutex.Header
        let m = HeaderedMutex::new("/locks/k");
        let h = m.lock(100).unwrap();
        assert_eq!(h.lease_id, 100);
        assert_eq!(h.key, "/locks/k");
        assert!(h.acquired_revision >= 1);
    }

    #[test]
    fn test_header_age_increases() {
        // cite: mutex.go (acquired_at frozen at acquisition)
        let m = HeaderedMutex::new("/k");
        let h = m.lock(1).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(h.age() >= Duration::from_millis(5));
    }

    #[test]
    fn test_header_persisted_in_holder() {
        let m = HeaderedMutex::new("/k");
        m.lock(1).unwrap();
        assert!(m.header().is_some());
    }

    #[test]
    fn test_header_cleared_on_unlock_with_no_waiter() {
        // cite: mutex.go (Unlock + empty queue ⇒ no holder)
        let m = HeaderedMutex::new("/k");
        m.lock(1).unwrap();
        m.unlock(1).unwrap();
        assert!(m.header().is_none());
    }

    #[test]
    fn test_header_promoted_to_next_waiter_on_unlock() {
        // cite: mutex.go (Unlock ⇒ next waiter becomes holder + Header)
        let m = HeaderedMutex::new("/k");
        m.lock(1).unwrap();
        let _ = m.lock(2);
        m.unlock(1).unwrap();
        assert_eq!(m.header().unwrap().lease_id, 2);
    }

    #[test]
    fn test_reentrant_returns_same_header() {
        // cite: mutex.go (Lock under same lease is idempotent)
        let m = HeaderedMutex::new("/k");
        let h1 = m.lock(1).unwrap();
        let h2 = m.lock(1).unwrap();
        assert_eq!(h1.acquired_revision, h2.acquired_revision);
    }

    #[test]
    fn test_unlock_non_owner_errors() {
        let m = HeaderedMutex::new("/k");
        m.lock(1).unwrap();
        assert_eq!(m.unlock(99).unwrap_err(), LockManagerError::NotOwner);
    }

    #[test]
    fn test_unlock_when_free_errors() {
        let m = HeaderedMutex::new("/k");
        assert_eq!(m.unlock(1).unwrap_err(), LockManagerError::NotOwner);
    }

    #[test]
    fn test_lock_acquired_revision_monotonic() {
        // cite: mvcc revisions monotonic
        let m = HeaderedMutex::new("/k");
        let r1 = m.lock(1).unwrap().acquired_revision;
        m.unlock(1).unwrap();
        let r2 = m.lock(2).unwrap().acquired_revision;
        assert!(r2 > r1);
    }

    // ── try_lock_until ────────────────────────────────────────────────

    #[test]
    fn test_try_lock_until_succeeds_when_free() {
        // cite: recipes/double_barrier.go (deadline acquire)
        let m = HeaderedMutex::new("/k");
        let h = m
            .try_lock_until(7, Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert_eq!(h.lease_id, 7);
    }

    #[test]
    fn test_try_lock_until_deadline_exceeded() {
        // cite: recipes (deadline path)
        let m = HeaderedMutex::new("/k");
        m.lock(1).unwrap();
        let err = m
            .try_lock_until(2, Instant::now() + Duration::from_millis(20))
            .unwrap_err();
        assert_eq!(err, LockManagerError::DeadlineExceeded);
    }

    #[test]
    fn test_try_lock_until_promoted_after_holder_releases() {
        // cite: recipes (waiter wakes up on release)
        use std::sync::Arc;
        let m = Arc::new(HeaderedMutex::new("/k"));
        m.lock(1).unwrap();
        let m2 = m.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            m2.unlock(1).unwrap();
        });
        let h = m
            .try_lock_until(2, Instant::now() + Duration::from_millis(200))
            .unwrap();
        t.join().unwrap();
        assert_eq!(h.lease_id, 2);
    }

    // ── acquire_all (multi-lock) ──────────────────────────────────────

    #[test]
    fn test_acquire_all_succeeds_in_sorted_order() {
        // cite: classic deadlock-avoidance pattern
        let locks = three_locks();
        let keys = vec!["/c".into(), "/a".into(), "/b".into()];
        let headers = acquire_all(&locks, &keys, 1).unwrap();
        // Result preserves request order.
        assert_eq!(headers[0].key, "/c");
        assert_eq!(headers[1].key, "/a");
        assert_eq!(headers[2].key, "/b");
    }

    #[test]
    fn test_acquire_all_duplicate_key_errors() {
        // cite: defensive against double-acquire
        let locks = three_locks();
        let keys = vec!["/a".into(), "/a".into()];
        match acquire_all(&locks, &keys, 1).unwrap_err() {
            LockManagerError::DuplicateKey(k) => assert_eq!(k, "/a"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_acquire_all_unknown_key_errors() {
        let locks = three_locks();
        let keys = vec!["/x".into()];
        match acquire_all(&locks, &keys, 1).unwrap_err() {
            LockManagerError::UnknownLock(k) => assert_eq!(k, "/x"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_acquire_all_releases_all_in_release_call() {
        let locks = three_locks();
        let keys = vec!["/a".into(), "/b".into()];
        acquire_all(&locks, &keys, 1).unwrap();
        release_all(&locks, &keys, 1).unwrap();
        assert!(locks.get("/a").unwrap().header().is_none());
        assert!(locks.get("/b").unwrap().header().is_none());
    }

    #[test]
    fn test_release_all_unknown_key_errors() {
        let locks = three_locks();
        assert!(release_all(&locks, &["/ghost".into()], 1).is_err());
    }

    // ── LockManager registry ──────────────────────────────────────────

    #[test]
    fn test_lock_manager_get_or_create_idempotent() {
        // cite: client/v3/concurrency NewMutex(prefix)
        let m = LockManager::new();
        m.get_or_create("/lock-a");
        m.get_or_create("/lock-a");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn test_lock_manager_lock_unlock_roundtrip() {
        let m = LockManager::new();
        m.get_or_create("/k");
        let h = m.lock("/k", 7).unwrap();
        assert_eq!(h.lease_id, 7);
        m.unlock("/k", 7).unwrap();
        assert!(m.header("/k").is_none());
    }

    #[test]
    fn test_lock_manager_unknown_lock_errors() {
        let m = LockManager::new();
        assert!(matches!(
            m.lock("/x", 1).unwrap_err(),
            LockManagerError::UnknownLock(_)
        ));
    }

    #[test]
    fn test_lock_manager_names_sorted() {
        let m = LockManager::new();
        m.get_or_create("/c");
        m.get_or_create("/a");
        m.get_or_create("/b");
        assert_eq!(m.names(), vec!["/a", "/b", "/c"]);
    }

    #[test]
    fn test_lock_manager_drop_lock() {
        let m = LockManager::new();
        m.get_or_create("/k");
        assert!(m.drop_lock("/k"));
        assert!(m.is_empty());
    }

    #[test]
    fn test_lock_manager_drop_unknown_returns_false() {
        let m = LockManager::new();
        assert!(!m.drop_lock("/ghost"));
    }

    #[test]
    fn test_lock_manager_header_for_unknown_returns_none() {
        let m = LockManager::new();
        assert!(m.header("/ghost").is_none());
    }

    // ── EpochElection ─────────────────────────────────────────────────

    #[test]
    fn test_epoch_election_first_campaign_becomes_leader() {
        // cite: client/v3/concurrency/election.go Campaign
        let e = EpochElection::new();
        let (_, became) = e.campaign(7, b"node-A".to_vec());
        assert!(became);
        assert_eq!(e.current_leader().unwrap().lease_id, 7);
        assert_eq!(e.current_epoch(), Some(1));
    }

    #[test]
    fn test_epoch_increases_across_transitions() {
        // cite: election.go (Resign ⇒ new epoch)
        let e = EpochElection::new();
        e.campaign(1, b"a".to_vec());
        e.campaign(2, b"b".to_vec());
        let ep1 = e.current_epoch().unwrap();
        e.resign(1).unwrap();
        let ep2 = e.current_epoch().unwrap();
        assert!(ep2 > ep1);
    }

    #[test]
    fn test_epoch_election_proclaim_updates_value() {
        // cite: election.go Proclaim
        let e = EpochElection::new();
        e.campaign(7, b"v0".to_vec());
        e.proclaim(7, b"v1".to_vec()).unwrap();
        assert_eq!(e.current_leader().unwrap().value, b"v1");
    }

    #[test]
    fn test_epoch_election_proclaim_non_leader_errors() {
        let e = EpochElection::new();
        e.campaign(1, b"v".to_vec());
        e.campaign(2, b"v".to_vec());
        assert_eq!(
            e.proclaim(2, b"x".to_vec()).unwrap_err(),
            LockManagerError::NotLeader
        );
    }

    #[test]
    fn test_epoch_election_proclaim_no_leader_errors() {
        let e = EpochElection::new();
        assert_eq!(
            e.proclaim(1, b"x".to_vec()).unwrap_err(),
            LockManagerError::NoLeader
        );
    }

    #[test]
    fn test_epoch_election_resign_when_no_leader() {
        let e = EpochElection::new();
        assert_eq!(e.resign(1).unwrap_err(), LockManagerError::NoLeader);
    }

    #[test]
    fn test_epoch_election_resign_non_leader_errors() {
        let e = EpochElection::new();
        e.campaign(1, b"a".to_vec());
        e.campaign(2, b"b".to_vec());
        assert_eq!(e.resign(2).unwrap_err(), LockManagerError::NotLeader);
    }

    #[test]
    fn test_epoch_election_resign_promotes_next_with_new_epoch() {
        // cite: election.go Resign + Observe
        let e = EpochElection::new();
        e.campaign(1, b"a".to_vec());
        e.campaign(2, b"b".to_vec());
        let new = e.resign(1).unwrap().unwrap();
        assert_eq!(new.1, 2);
        assert!(new.0 > 1);
    }

    #[test]
    fn test_epoch_election_resign_last_returns_none() {
        let e = EpochElection::new();
        e.campaign(1, b"a".to_vec());
        assert_eq!(e.resign(1).unwrap(), None);
        assert!(e.current_leader().is_none());
    }

    #[test]
    fn test_epoch_election_observer_records_transitions() {
        // cite: election.go Observe channel
        let e = EpochElection::new();
        e.campaign(1, b"a".to_vec());
        e.campaign(2, b"b".to_vec());
        e.resign(1).unwrap();
        let hist = e.observer_history();
        // first record: 1 elected, second: 2 elected after resign
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].lease_id, 1);
        assert_eq!(hist[1].lease_id, 2);
        assert!(hist[1].epoch > hist[0].epoch);
    }

    #[test]
    fn test_epoch_election_observer_buffer_capped() {
        // cite: bounded ring buffer (no unbounded growth)
        let e = EpochElection::new().with_observer_cap(2);
        for n in 1..=4i64 {
            e.campaign(n, b"x".to_vec());
            if n > 1 {
                e.resign(n - 1).unwrap();
            }
        }
        assert_eq!(e.observer_history().len(), 2);
    }

    #[test]
    fn test_epoch_election_lease_expire_promotes_with_new_epoch() {
        // cite: election.go (lease revoke ⇒ leader changes)
        let e = EpochElection::new();
        e.campaign(1, b"a".to_vec());
        e.campaign(2, b"b".to_vec());
        let prev_epoch = e.current_epoch().unwrap();
        let new = e.expire_lease(1).unwrap();
        assert_eq!(new.1, 2);
        assert!(new.0 > prev_epoch);
    }

    #[test]
    fn test_epoch_election_lease_expire_non_leader_no_change() {
        // cite: election.go (only leader's expiry triggers promotion)
        let e = EpochElection::new();
        e.campaign(1, b"a".to_vec());
        e.campaign(2, b"b".to_vec());
        let before = e.current_epoch();
        let res = e.expire_lease(2);
        assert!(res.is_none());
        assert_eq!(e.current_epoch(), before);
        assert_eq!(e.queue_len(), 1);
    }

    #[test]
    fn test_epoch_election_value_carries_through_observe() {
        let e = EpochElection::new();
        e.campaign(7, b"hostname-a".to_vec());
        let h = e.observer_history();
        assert_eq!(h[0].value, b"hostname-a");
    }

    #[test]
    fn test_epoch_election_queue_len_tracks_candidates() {
        let e = EpochElection::new();
        for n in 1..=5i64 {
            e.campaign(n, b"x".to_vec());
        }
        assert_eq!(e.queue_len(), 5);
    }

    // ── Mixed integration ────────────────────────────────────────────

    #[test]
    fn test_mutex_promote_does_not_lose_queue() {
        // cite: mutex.go (waiters survive holder release)
        let m = HeaderedMutex::new("/k");
        m.lock(1).unwrap();
        let _ = m.lock(2);
        let _ = m.lock(3);
        m.unlock(1).unwrap();
        assert_eq!(m.queue_len(), 2);
        assert_eq!(m.header().unwrap().lease_id, 2);
    }

    #[test]
    fn test_mutex_unlock_returns_ok_when_holder() {
        let m = HeaderedMutex::new("/k");
        m.lock(7).unwrap();
        assert!(m.unlock(7).is_ok());
    }

    #[test]
    fn test_acquire_all_with_empty_keys_succeeds() {
        // cite: defensive — empty batch is a no-op
        let locks = three_locks();
        let h = acquire_all(&locks, &[], 1).unwrap();
        assert!(h.is_empty());
    }

    #[test]
    fn test_release_all_with_empty_keys_succeeds() {
        let locks = three_locks();
        assert!(release_all(&locks, &[], 1).is_ok());
    }

    #[test]
    fn test_acquire_all_then_release_all_is_idempotent_no_double_release() {
        let locks = three_locks();
        let keys = vec!["/a".into()];
        acquire_all(&locks, &keys, 1).unwrap();
        release_all(&locks, &keys, 1).unwrap();
        // Second release ⇒ NotOwner.
        assert!(release_all(&locks, &keys, 1).is_err());
    }

    #[test]
    fn test_lock_manager_concurrent_locks_distinct() {
        let m = LockManager::new();
        m.get_or_create("/a");
        m.get_or_create("/b");
        m.lock("/a", 1).unwrap();
        m.lock("/b", 2).unwrap();
        assert_eq!(m.header("/a").unwrap().lease_id, 1);
        assert_eq!(m.header("/b").unwrap().lease_id, 2);
    }

    #[test]
    fn test_epoch_election_first_epoch_is_one() {
        // cite: election.go (epoch starts at 1)
        let e = EpochElection::new();
        e.campaign(1, b"a".to_vec());
        assert_eq!(e.current_leader().unwrap().epoch, 1);
    }

    #[test]
    fn test_epoch_election_resign_chain() {
        // cite: election.go (chain of resigns)
        let e = EpochElection::new();
        for n in 1..=4i64 {
            e.campaign(n, b"x".to_vec());
        }
        for n in 1..=3i64 {
            e.resign(n).unwrap();
        }
        assert_eq!(e.current_leader().unwrap().lease_id, 4);
    }

    #[test]
    fn test_epoch_election_proclaim_keeps_epoch() {
        // cite: election.go (Proclaim is value-only, epoch unchanged)
        let e = EpochElection::new();
        e.campaign(7, b"v0".to_vec());
        let ep = e.current_epoch().unwrap();
        e.proclaim(7, b"v1".to_vec()).unwrap();
        assert_eq!(e.current_epoch(), Some(ep));
    }
}
