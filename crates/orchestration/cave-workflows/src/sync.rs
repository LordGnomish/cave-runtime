// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Workflow synchronization — port of `argoproj/argo-workflows`
//! `workflow/sync` SyncManager (counting semaphores + binary mutexes).
//!
//! A workflow (or one of its nodes) declares a `synchronization` reference to
//! a named semaphore or mutex. The [`SyncManager`] gates concurrent access:
//! a counting semaphore admits up to `limit` holders; a mutex is a semaphore
//! of limit 1. Waiters queue ordered by workflow **priority** (higher first)
//! then **creation time** (earlier first), so acquisition is fair FIFO within
//! a priority level. The holder key is `namespace/workflow[/node]`.
//!
//! RBAC / access-control for *who may use* a lock stays in `cave-permission`;
//! this module owns the lock algorithm only.

use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap};

/// Lock flavour declared by a workflow's `synchronization` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncRef {
    Semaphore { name: String, limit: usize },
    Mutex { name: String },
}

impl SyncRef {
    fn name(&self) -> &str {
        match self {
            SyncRef::Semaphore { name, .. } => name,
            SyncRef::Mutex { name } => name,
        }
    }
    fn limit(&self) -> usize {
        match self {
            SyncRef::Semaphore { limit, .. } => *limit,
            SyncRef::Mutex { .. } => 1,
        }
    }
}

/// Result of a [`SyncManager::try_acquire`] attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcquireResult {
    Acquired,
    Waiting(String),
}

impl AcquireResult {
    pub fn is_acquired(&self) -> bool {
        matches!(self, AcquireResult::Acquired)
    }
}

/// Build the holder key `namespace/workflow[/node]`.
pub fn holder_key(namespace: &str, workflow: &str, node: Option<&str>) -> String {
    match node {
        Some(n) if !n.is_empty() => format!("{namespace}/{workflow}/{n}"),
        _ => format!("{namespace}/{workflow}"),
    }
}

#[derive(Clone, Debug)]
struct QueueItem {
    key: String,
    priority: i32,
    creation: DateTime<Utc>,
}

#[derive(Debug)]
struct Lock {
    limit: usize,
    holders: BTreeSet<String>,
    queue: Vec<QueueItem>,
}

impl Lock {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            holders: BTreeSet::new(),
            queue: Vec::new(),
        }
    }

    /// Enqueue a request if it is neither already queued nor already holding.
    fn add_to_queue(&mut self, key: &str, priority: i32, creation: DateTime<Utc>) {
        if self.holders.contains(key) || self.queue.iter().any(|q| q.key == key) {
            return;
        }
        self.queue.push(QueueItem {
            key: key.to_string(),
            priority,
            creation,
        });
    }

    /// Try to acquire for `key`. Succeeds if already holding, or if `key` is
    /// at the front of the priority-ordered queue and capacity remains.
    fn try_acquire(&mut self, key: &str) -> bool {
        if self.holders.contains(key) {
            return true;
        }
        if self.holders.len() >= self.limit {
            return false;
        }
        // Order: higher priority first, then earlier creation, then key.
        self.queue.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then(a.creation.cmp(&b.creation))
                .then(a.key.cmp(&b.key))
        });
        match self.queue.first() {
            Some(front) if front.key == key => {
                let k = front.key.clone();
                self.queue.retain(|q| q.key != k);
                self.holders.insert(k);
                true
            }
            _ => false,
        }
    }

    fn release(&mut self, key: &str) {
        self.holders.remove(key);
        self.queue.retain(|q| q.key != key);
    }
}

/// Lock manager mirroring Argo's `SyncManager`.
#[derive(Debug, Default)]
pub struct SyncManager {
    locks: HashMap<String, Lock>,
}

impl SyncManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue `holder_key` for the referenced lock and attempt acquisition.
    /// Idempotent: re-calling for a key that already holds returns
    /// `Acquired` without changing the holder count.
    pub fn try_acquire(
        &mut self,
        sync: &SyncRef,
        holder_key: &str,
        priority: i32,
        creation: DateTime<Utc>,
    ) -> AcquireResult {
        let name = sync.name().to_string();
        let limit = sync.limit();
        let lock = self.locks.entry(name.clone()).or_insert_with(|| Lock::new(limit));
        // A semaphore's size may change between reconciles.
        lock.limit = limit;
        lock.add_to_queue(holder_key, priority, creation);
        if lock.try_acquire(holder_key) {
            AcquireResult::Acquired
        } else {
            AcquireResult::Waiting(format!(
                "Waiting for {name} lock. Lock status: {}/{}",
                lock.holders.len(),
                lock.limit
            ))
        }
    }

    /// Release a single holder from a named lock.
    pub fn release(&mut self, lock_name: &str, holder_key: &str) {
        if let Some(lock) = self.locks.get_mut(lock_name) {
            lock.release(holder_key);
        }
    }

    /// Release every hold and queued request belonging to a workflow
    /// (`namespace/workflow` prefix), across all locks.
    pub fn release_all(&mut self, workflow_prefix: &str) {
        let matches = |key: &str| key == workflow_prefix || key.starts_with(&format!("{workflow_prefix}/"));
        for lock in self.locks.values_mut() {
            lock.holders.retain(|h| !matches(h));
            lock.queue.retain(|q| !matches(&q.key));
        }
    }

    /// Number of current holders of a lock (0 if unknown).
    pub fn holder_count(&self, lock_name: &str) -> usize {
        self.locks.get(lock_name).map(|l| l.holders.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_780_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn holder_key_with_and_without_node() {
        assert_eq!(holder_key("ns", "wf", Some("step-1")), "ns/wf/step-1");
        assert_eq!(holder_key("ns", "wf", None), "ns/wf");
        assert_eq!(holder_key("ns", "wf", Some("")), "ns/wf");
    }

    #[test]
    fn mutex_admits_one_holder() {
        let mut m = SyncManager::new();
        let mx = SyncRef::Mutex { name: "db".into() };
        let a = m.try_acquire(&mx, "ns/wf-a", 0, ts(0));
        let b = m.try_acquire(&mx, "ns/wf-b", 0, ts(1));
        assert!(a.is_acquired());
        assert!(!b.is_acquired(), "second holder must wait");
        assert_eq!(m.holder_count("db"), 1);
    }

    #[test]
    fn release_lets_next_waiter_acquire() {
        let mut m = SyncManager::new();
        let mx = SyncRef::Mutex { name: "db".into() };
        assert!(m.try_acquire(&mx, "ns/wf-a", 0, ts(0)).is_acquired());
        assert!(!m.try_acquire(&mx, "ns/wf-b", 0, ts(1)).is_acquired());
        m.release("db", "ns/wf-a");
        // b retries and now gets it.
        assert!(m.try_acquire(&mx, "ns/wf-b", 0, ts(1)).is_acquired());
        assert_eq!(m.holder_count("db"), 1);
    }

    #[test]
    fn semaphore_admits_up_to_limit() {
        let mut m = SyncManager::new();
        let s = SyncRef::Semaphore { name: "pool".into(), limit: 2 };
        assert!(m.try_acquire(&s, "ns/a", 0, ts(0)).is_acquired());
        assert!(m.try_acquire(&s, "ns/b", 0, ts(1)).is_acquired());
        assert!(!m.try_acquire(&s, "ns/c", 0, ts(2)).is_acquired(), "3rd over limit waits");
        assert_eq!(m.holder_count("pool"), 2);
    }

    #[test]
    fn idempotent_acquire_does_not_double_count() {
        let mut m = SyncManager::new();
        let mx = SyncRef::Mutex { name: "db".into() };
        assert!(m.try_acquire(&mx, "ns/wf-a", 0, ts(0)).is_acquired());
        assert!(m.try_acquire(&mx, "ns/wf-a", 0, ts(0)).is_acquired(), "re-acquire same key ok");
        assert_eq!(m.holder_count("db"), 1);
    }

    #[test]
    fn higher_priority_waiter_jumps_the_queue() {
        let mut m = SyncManager::new();
        let mx = SyncRef::Mutex { name: "db".into() };
        // Holder takes the lock.
        assert!(m.try_acquire(&mx, "ns/holder", 0, ts(0)).is_acquired());
        // Low-priority waiter enqueues first, then a high-priority waiter.
        assert!(!m.try_acquire(&mx, "ns/low", 1, ts(1)).is_acquired());
        assert!(!m.try_acquire(&mx, "ns/high", 9, ts(2)).is_acquired());
        m.release("db", "ns/holder");
        // High priority must win despite enqueuing later.
        assert!(!m.try_acquire(&mx, "ns/low", 1, ts(1)).is_acquired(), "low still waits");
        assert!(m.try_acquire(&mx, "ns/high", 9, ts(2)).is_acquired());
    }

    #[test]
    fn same_priority_is_fifo_by_creation_time() {
        let mut m = SyncManager::new();
        let mx = SyncRef::Mutex { name: "db".into() };
        assert!(m.try_acquire(&mx, "ns/holder", 0, ts(0)).is_acquired());
        // Same priority; earlier creation should win.
        assert!(!m.try_acquire(&mx, "ns/later", 5, ts(20)).is_acquired());
        assert!(!m.try_acquire(&mx, "ns/earlier", 5, ts(10)).is_acquired());
        m.release("db", "ns/holder");
        assert!(!m.try_acquire(&mx, "ns/later", 5, ts(20)).is_acquired(), "later still waits");
        assert!(m.try_acquire(&mx, "ns/earlier", 5, ts(10)).is_acquired());
    }

    #[test]
    fn release_all_clears_workflow_holds_and_queue() {
        let mut m = SyncManager::new();
        let mx = SyncRef::Mutex { name: "db".into() };
        let sem = SyncRef::Semaphore { name: "pool".into(), limit: 1 };
        // wf-a holds both via two nodes.
        assert!(m.try_acquire(&mx, "ns/wf-a/n1", 0, ts(0)).is_acquired());
        assert!(m.try_acquire(&sem, "ns/wf-a/n2", 0, ts(1)).is_acquired());
        // wf-b waits on the mutex.
        assert!(!m.try_acquire(&mx, "ns/wf-b/n1", 0, ts(2)).is_acquired());
        // Release everything for wf-a.
        m.release_all("ns/wf-a");
        assert_eq!(m.holder_count("db"), 0);
        assert_eq!(m.holder_count("pool"), 0);
        // wf-b can now take the mutex.
        assert!(m.try_acquire(&mx, "ns/wf-b/n1", 0, ts(2)).is_acquired());
    }

    #[test]
    fn waiting_message_names_the_lock() {
        let mut m = SyncManager::new();
        let mx = SyncRef::Mutex { name: "db".into() };
        m.try_acquire(&mx, "ns/holder", 0, ts(0));
        match m.try_acquire(&mx, "ns/waiter", 0, ts(1)) {
            AcquireResult::Waiting(msg) => assert!(msg.contains("db"), "msg: {msg}"),
            AcquireResult::Acquired => panic!("expected waiting"),
        }
    }
}
