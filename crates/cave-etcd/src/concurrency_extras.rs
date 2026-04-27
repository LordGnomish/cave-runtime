//! Higher-level concurrency primitives layered on top of
//! [`crate::concurrency`]: `Session`, `Barrier`, `PriorityQueue`, and an
//! STM (software transactional memory) abstraction.
//!
//! Mirrors etcd v3.6.10
//!   `client/v3/concurrency/session.go` — lease-bound session shared by
//!     mutexes/elections,
//!   `client/v3/recipes/barrier.go` — N-way barrier,
//!   `client/v3/recipes/priority_queue.go` — etcd-as-MQ with priority,
//!   `client/v3/concurrency/stm.go` — repeatable-read STM.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

// ── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    SessionDone,
    LeaseInvalid,
    BarrierAlreadyHeld,
    BarrierNotHeld,
    QueueEmpty,
    StmConflict,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionDone => write!(f, "session done"),
            Self::LeaseInvalid => write!(f, "lease invalid"),
            Self::BarrierAlreadyHeld => write!(f, "barrier already held"),
            Self::BarrierNotHeld => write!(f, "barrier not held"),
            Self::QueueEmpty => write!(f, "queue empty"),
            Self::StmConflict => write!(f, "stm conflict"),
        }
    }
}

impl std::error::Error for SessionError {}

// ── Session ──────────────────────────────────────────────────────────────

/// A lease-bound session.  Owns a lease id; concurrency primitives that
/// share the same session also share its lifetime — when the session is
/// `done()`'d, every primitive bound to it gets a "lease expired" signal.
pub struct Session {
    lease_id: i64,
    ttl_secs: i64,
    done: Mutex<bool>,
    /// Hooks fired when the session ends — used by mutex/election to release
    /// keys.  Each hook is called exactly once.
    expire_hooks: Mutex<Vec<Box<dyn Fn(i64) + Send + Sync>>>,
}

impl Session {
    pub fn new(lease_id: i64, ttl_secs: i64) -> Self {
        Self {
            lease_id, ttl_secs,
            done: Mutex::new(false),
            expire_hooks: Mutex::new(Vec::new()),
        }
    }

    pub fn lease_id(&self) -> i64 { self.lease_id }
    pub fn ttl_secs(&self) -> i64 { self.ttl_secs }
    pub fn is_done(&self) -> bool { *self.done.lock().unwrap() }

    /// Register a hook to run when [`Self::done`] is called.
    pub fn on_expire(&self, hook: impl Fn(i64) + Send + Sync + 'static) {
        self.expire_hooks.lock().unwrap().push(Box::new(hook));
    }

    /// End the session — fires every registered hook in registration order.
    pub fn done(&self) -> Result<(), SessionError> {
        let mut d = self.done.lock().unwrap();
        if *d { return Err(SessionError::SessionDone); }
        *d = true;
        let hooks = std::mem::take(&mut *self.expire_hooks.lock().unwrap());
        let id = self.lease_id;
        drop(d);
        for h in hooks { h(id); }
        Ok(())
    }
}

// ── Barrier ──────────────────────────────────────────────────────────────

/// N-way barrier: every participant calls `wait()`; once `count` of them
/// have arrived, the barrier "trips" and all subsequent `wait()`s pass
/// through immediately.  Holding the barrier is signalled by `hold()`.
pub struct Barrier {
    target: usize,
    state: Mutex<BarrierState>,
}

#[derive(Default)]
struct BarrierState {
    waiters: usize,
    held: bool,
}

impl Barrier {
    pub fn new(target: usize) -> Self {
        Self { target, state: Mutex::new(BarrierState::default()) }
    }

    /// Place the barrier in the held state.  Returns `BarrierAlreadyHeld`
    /// if it's already held.  Mirrors etcd's `Barrier.Hold`.
    pub fn hold(&self) -> Result<(), SessionError> {
        let mut s = self.state.lock().unwrap();
        if s.held { return Err(SessionError::BarrierAlreadyHeld); }
        s.held = true;
        Ok(())
    }

    /// Release the barrier — every queued/future `wait()` returns Ok.
    pub fn release(&self) -> Result<(), SessionError> {
        let mut s = self.state.lock().unwrap();
        if !s.held { return Err(SessionError::BarrierNotHeld); }
        s.held = false;
        Ok(())
    }

    /// Test-friendly synchronous wait — returns `true` once enough waiters
    /// have arrived and the barrier is released.  Increments the waiter
    /// counter unless `released` already.
    pub fn wait(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        s.waiters += 1;
        if !s.held && s.waiters >= self.target {
            return true;
        }
        false
    }

    pub fn waiters(&self) -> usize { self.state.lock().unwrap().waiters }
    pub fn is_held(&self) -> bool { self.state.lock().unwrap().held }
}

// ── PriorityQueue ────────────────────────────────────────────────────────

/// Items in a priority queue carry `priority` (lower wins) and
/// `enqueued_at` (FIFO tiebreaker).
#[derive(Debug, Clone)]
pub struct PqItem {
    pub priority: u64,
    pub enqueued_at: u64,
    pub value: Vec<u8>,
}

/// Priority queue: smallest priority, then FIFO.
pub struct PriorityQueue {
    next: AtomicU64,
    inner: Mutex<BTreeMap<(u64, u64), Vec<u8>>>,
}

impl PriorityQueue {
    pub fn new() -> Self { Self { next: AtomicU64::new(0), inner: Mutex::new(BTreeMap::new()) } }

    pub fn enqueue(&self, priority: u64, value: Vec<u8>) -> u64 {
        let seq = self.next.fetch_add(1, Ordering::SeqCst);
        self.inner.lock().unwrap().insert((priority, seq), value);
        seq
    }

    pub fn dequeue(&self) -> Result<PqItem, SessionError> {
        let mut q = self.inner.lock().unwrap();
        let key = q.keys().next().cloned().ok_or(SessionError::QueueEmpty)?;
        let value = q.remove(&key).unwrap();
        Ok(PqItem { priority: key.0, enqueued_at: key.1, value })
    }

    pub fn peek(&self) -> Option<PqItem> {
        self.inner.lock().unwrap().iter().next().map(|((p, e), v)| PqItem {
            priority: *p, enqueued_at: *e, value: v.clone(),
        })
    }

    pub fn len(&self) -> usize { self.inner.lock().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

impl Default for PriorityQueue {
    fn default() -> Self { Self::new() }
}

// ── STM ──────────────────────────────────────────────────────────────────

/// Versioned key — STM observes the version of every key it reads, then
/// retries if any of those versions changed at commit time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedValue {
    pub value: Vec<u8>,
    pub version: u64,
}

/// In-memory STM (repeatable-read with optimistic concurrency).  Multiple
/// transactions can run; on `commit` the runtime checks that every key
/// the transaction read still has the version it observed; if any changed
/// the transaction returns `StmConflict` and the caller retries.
pub struct StmStore {
    state: Mutex<HashMap<String, VersionedValue>>,
    /// Counts every committed transaction.
    commits: AtomicU64,
    /// Counts every conflict observed.
    conflicts: AtomicU64,
}

impl StmStore {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            commits: AtomicU64::new(0),
            conflicts: AtomicU64::new(0),
        }
    }

    pub fn put_unchecked(&self, key: &str, value: Vec<u8>) {
        let mut s = self.state.lock().unwrap();
        let entry = s.entry(key.to_string()).or_insert(VersionedValue { value: vec![], version: 0 });
        entry.value = value;
        entry.version += 1;
    }

    pub fn get_versioned(&self, key: &str) -> Option<VersionedValue> {
        self.state.lock().unwrap().get(key).cloned()
    }

    /// Open a new transaction.
    pub fn begin(&self) -> StmTxn<'_> {
        StmTxn { store: self, reads: HashMap::new(), writes: HashMap::new() }
    }

    pub fn commits(&self) -> u64 { self.commits.load(Ordering::SeqCst) }
    pub fn conflicts(&self) -> u64 { self.conflicts.load(Ordering::SeqCst) }
}

impl Default for StmStore {
    fn default() -> Self { Self::new() }
}

/// One in-flight STM transaction.
pub struct StmTxn<'a> {
    store: &'a StmStore,
    reads: HashMap<String, u64>,
    writes: HashMap<String, Vec<u8>>,
}

impl StmTxn<'_> {
    /// Read a key — caches the version so commit can detect changes.
    pub fn get(&mut self, key: &str) -> Option<Vec<u8>> {
        if let Some(buf) = self.writes.get(key) { return Some(buf.clone()); }
        let s = self.store.state.lock().unwrap();
        let entry = s.get(key)?;
        let v = entry.value.clone();
        let version = entry.version;
        drop(s);
        self.reads.insert(key.to_string(), version);
        Some(v)
    }

    /// Stage a write.  Not visible to other transactions until commit.
    pub fn put(&mut self, key: &str, value: Vec<u8>) {
        self.writes.insert(key.to_string(), value);
    }

    /// Attempt to commit.  Aborts with `StmConflict` if any read version
    /// changed in the meantime.
    pub fn commit(self) -> Result<(), SessionError> {
        let mut s = self.store.state.lock().unwrap();
        // Validate read set.
        for (k, ver_at_read) in self.reads.iter() {
            let now = s.get(k).map(|e| e.version).unwrap_or(0);
            if now != *ver_at_read {
                self.store.conflicts.fetch_add(1, Ordering::SeqCst);
                return Err(SessionError::StmConflict);
            }
        }
        // Apply writes atomically.
        for (k, v) in self.writes.iter() {
            let entry = s.entry(k.clone()).or_insert(VersionedValue { value: vec![], version: 0 });
            entry.value = v.clone();
            entry.version += 1;
        }
        self.store.commits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Abort without committing.
    pub fn abort(self) {}

    pub fn reads(&self) -> &HashMap<String, u64> { &self.reads }
    pub fn writes(&self) -> &HashMap<String, Vec<u8>> { &self.writes }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M10
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    // ── Session ───────────────────────────────────────────────────────

    #[test]
    fn test_session_done_runs_hooks_once() {
        // cite: client/v3/concurrency/session.go (Session.Close)
        let s = Session::new(42, 30);
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        s.on_expire(move |id| {
            assert_eq!(id, 42);
            c.fetch_add(1, Ordering::SeqCst);
        });
        s.done().unwrap();
        assert!(s.is_done());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_session_done_twice_errors() {
        // cite: session.go (close-after-close ⇒ ErrSessionDone)
        let s = Session::new(1, 30);
        s.done().unwrap();
        assert_eq!(s.done().unwrap_err(), SessionError::SessionDone);
    }

    #[test]
    fn test_session_runs_hooks_in_order() {
        // cite: session.go (Close fires hooks LIFO/FIFO consistently)
        let s = Session::new(1, 30);
        let order = Arc::new(Mutex::new(Vec::<u32>::new()));
        for n in 0..5u32 {
            let order = order.clone();
            s.on_expire(move |_| order.lock().unwrap().push(n));
        }
        s.done().unwrap();
        assert_eq!(*order.lock().unwrap(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_session_metadata_round_trip() {
        let s = Session::new(7, 60);
        assert_eq!(s.lease_id(), 7);
        assert_eq!(s.ttl_secs(), 60);
        assert!(!s.is_done());
    }

    // ── Barrier ───────────────────────────────────────────────────────

    #[test]
    fn test_barrier_holds_then_releases() {
        // cite: recipes/barrier.go (Hold + Release)
        let b = Barrier::new(2);
        b.hold().unwrap();
        assert!(b.is_held());
        b.release().unwrap();
        assert!(!b.is_held());
    }

    #[test]
    fn test_barrier_double_hold_errors() {
        // cite: recipes/barrier.go (already-held)
        let b = Barrier::new(1);
        b.hold().unwrap();
        assert_eq!(b.hold().unwrap_err(), SessionError::BarrierAlreadyHeld);
    }

    #[test]
    fn test_barrier_release_without_hold_errors() {
        // cite: recipes/barrier.go (release before hold ⇒ error)
        let b = Barrier::new(1);
        assert_eq!(b.release().unwrap_err(), SessionError::BarrierNotHeld);
    }

    #[test]
    fn test_barrier_wait_passes_when_target_reached() {
        // cite: recipes/barrier.go (wait succeeds at threshold)
        let b = Barrier::new(3);
        assert!(!b.wait()); // 1
        assert!(!b.wait()); // 2
        assert!(b.wait());  // 3 — trips
    }

    #[test]
    fn test_barrier_wait_blocked_while_held() {
        // cite: recipes/barrier.go (held ⇒ wait returns false)
        let b = Barrier::new(1);
        b.hold().unwrap();
        assert!(!b.wait());
    }

    #[test]
    fn test_barrier_waiters_counter() {
        let b = Barrier::new(10);
        for _ in 0..3 { b.wait(); }
        assert_eq!(b.waiters(), 3);
    }

    // ── PriorityQueue ─────────────────────────────────────────────────

    #[test]
    fn test_pq_dequeue_smallest_priority() {
        // cite: recipes/priority_queue.go (smallest key wins)
        let q = PriorityQueue::new();
        q.enqueue(5, b"five".to_vec());
        q.enqueue(1, b"one".to_vec());
        q.enqueue(3, b"three".to_vec());
        assert_eq!(q.dequeue().unwrap().value, b"one");
        assert_eq!(q.dequeue().unwrap().value, b"three");
        assert_eq!(q.dequeue().unwrap().value, b"five");
    }

    #[test]
    fn test_pq_fifo_within_priority() {
        // cite: recipes/priority_queue.go (FIFO tiebreaker by enqueue order)
        let q = PriorityQueue::new();
        q.enqueue(1, b"a".to_vec());
        q.enqueue(1, b"b".to_vec());
        q.enqueue(1, b"c".to_vec());
        assert_eq!(q.dequeue().unwrap().value, b"a");
        assert_eq!(q.dequeue().unwrap().value, b"b");
        assert_eq!(q.dequeue().unwrap().value, b"c");
    }

    #[test]
    fn test_pq_empty_dequeue_errors() {
        let q = PriorityQueue::new();
        assert_eq!(q.dequeue().unwrap_err(), SessionError::QueueEmpty);
    }

    #[test]
    fn test_pq_peek_does_not_remove() {
        let q = PriorityQueue::new();
        q.enqueue(1, b"x".to_vec());
        let _ = q.peek();
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn test_pq_len_and_is_empty() {
        let q = PriorityQueue::new();
        assert!(q.is_empty());
        q.enqueue(1, b"x".to_vec());
        assert_eq!(q.len(), 1);
        q.dequeue().unwrap();
        assert!(q.is_empty());
    }

    // ── STM ───────────────────────────────────────────────────────────

    #[test]
    fn test_stm_simple_read_modify_write() {
        // cite: concurrency/stm.go (basic txn)
        let s = StmStore::new();
        s.put_unchecked("k", b"old".to_vec());
        let mut t = s.begin();
        let v = t.get("k").unwrap();
        assert_eq!(v, b"old");
        t.put("k", b"new".to_vec());
        t.commit().unwrap();
        assert_eq!(s.get_versioned("k").unwrap().value, b"new");
    }

    #[test]
    fn test_stm_conflict_detection() {
        // cite: concurrency/stm.go (read-version mismatch ⇒ retry)
        let s = StmStore::new();
        s.put_unchecked("k", b"v0".to_vec());
        let mut t1 = s.begin();
        let _ = t1.get("k");
        // Concurrent writer bumps the version.
        s.put_unchecked("k", b"v1".to_vec());
        t1.put("k", b"from-t1".to_vec());
        assert_eq!(t1.commit().unwrap_err(), SessionError::StmConflict);
        // Conflict counter incremented.
        assert_eq!(s.conflicts(), 1);
    }

    #[test]
    fn test_stm_no_conflict_when_writes_dont_overlap_reads() {
        // cite: stm.go (only read-set drives conflict)
        let s = StmStore::new();
        s.put_unchecked("a", b"x".to_vec());
        s.put_unchecked("b", b"y".to_vec());
        let mut t = s.begin();
        let _ = t.get("a");
        // Concurrent writer touches an unrelated key.
        s.put_unchecked("b", b"y2".to_vec());
        t.put("a", b"new-a".to_vec());
        t.commit().unwrap();
    }

    #[test]
    fn test_stm_read_after_local_write_returns_local() {
        // cite: stm.go (txn writes shadow the read view)
        let s = StmStore::new();
        s.put_unchecked("k", b"db".to_vec());
        let mut t = s.begin();
        t.put("k", b"local".to_vec());
        assert_eq!(t.get("k"), Some(b"local".to_vec()));
    }

    #[test]
    fn test_stm_multiple_writes_apply_atomically() {
        // cite: stm.go (commit applies writes as a unit)
        let s = StmStore::new();
        let mut t = s.begin();
        t.put("a", b"1".to_vec());
        t.put("b", b"2".to_vec());
        t.put("c", b"3".to_vec());
        t.commit().unwrap();
        assert_eq!(s.get_versioned("a").unwrap().value, b"1");
        assert_eq!(s.get_versioned("b").unwrap().value, b"2");
        assert_eq!(s.get_versioned("c").unwrap().value, b"3");
    }

    #[test]
    fn test_stm_abort_does_not_persist() {
        // cite: stm.go (abort discards staged writes)
        let s = StmStore::new();
        let mut t = s.begin();
        t.put("k", b"never".to_vec());
        t.abort();
        assert!(s.get_versioned("k").is_none());
    }

    #[test]
    fn test_stm_commit_counter() {
        // cite: stm.go metrics: stm_commit_total
        let s = StmStore::new();
        for _ in 0..3 { let t = s.begin(); t.commit().unwrap(); }
        assert_eq!(s.commits(), 3);
    }

    #[test]
    fn test_stm_get_missing_key_returns_none() {
        let s = StmStore::new();
        let mut t = s.begin();
        assert!(t.get("missing").is_none());
    }

    #[test]
    fn test_stm_version_increments_on_each_write() {
        // cite: stm.go (version counter monotone)
        let s = StmStore::new();
        s.put_unchecked("k", b"v0".to_vec());
        let v0 = s.get_versioned("k").unwrap().version;
        s.put_unchecked("k", b"v1".to_vec());
        let v1 = s.get_versioned("k").unwrap().version;
        assert!(v1 > v0);
    }

    #[test]
    fn test_stm_reads_only_no_writes_does_not_bump_version() {
        // cite: stm.go (read-only commit is a no-op)
        let s = StmStore::new();
        s.put_unchecked("k", b"v".to_vec());
        let v = s.get_versioned("k").unwrap().version;
        let mut t = s.begin();
        let _ = t.get("k");
        t.commit().unwrap();
        assert_eq!(s.get_versioned("k").unwrap().version, v);
    }
}
