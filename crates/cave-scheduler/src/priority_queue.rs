// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Priority scheduling queue with active / backoff / unschedulable subqueues.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/internal/queue/scheduling_queue.go
//!   pkg/scheduler/apis/config/types.go (PriorityClass)
//!
//! Pods enter `active`. On scheduling failure due to a transient condition they go to
//! `backoff` for an exponential delay; on unrecoverable failure they go to
//! `unschedulable` and are flushed back to active by an explicit move() (e.g. when the
//! cluster snapshot changes).

use crate::framework::Pod;
use chrono::{DateTime, Duration, Utc};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// PriorityClass — non-preemptive priority value attached to a pod.
#[derive(Debug, Clone)]
pub struct PriorityClass {
    pub name: String,
    pub value: i32,
    pub global_default: bool,
    pub preemption_policy: PreemptionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreemptionPolicy {
    PreemptLowerPriority,
    Never,
}

#[derive(Debug, Clone)]
struct QueuedPod {
    pod: Pod,
    enqueued_at: DateTime<Utc>,
    backoff_until: Option<DateTime<Utc>>,
    #[allow(dead_code)] // mirrored from attempts_by_uid for snapshot/observability
    attempts: u32,
}

impl PartialEq for QueuedPod {
    fn eq(&self, other: &Self) -> bool {
        self.pod.uid == other.pod.uid
    }
}
impl Eq for QueuedPod {}
impl PartialOrd for QueuedPod {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for QueuedPod {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is max-heap → higher priority first; older pods first on tie.
        self.pod
            .spec
            .priority
            .cmp(&other.pod.spec.priority)
            .then_with(|| other.enqueued_at.cmp(&self.enqueued_at))
            .then_with(|| other.pod.uid.cmp(&self.pod.uid))
    }
}

pub struct PriorityQueue {
    active: BinaryHeap<QueuedPod>,
    backoff: Vec<QueuedPod>,
    unschedulable: HashMap<String, QueuedPod>,
    /// Persistent failure counter per pod uid — survives transitions between
    /// backoff and unschedulable so that exponential delay grows monotonically.
    attempts_by_uid: HashMap<String, u32>,
    initial_backoff: Duration,
    max_backoff: Duration,
}

impl PriorityQueue {
    pub fn new() -> Self {
        Self {
            active: BinaryHeap::new(),
            backoff: vec![],
            unschedulable: HashMap::new(),
            attempts_by_uid: HashMap::new(),
            initial_backoff: Duration::seconds(1),
            max_backoff: Duration::seconds(10),
        }
    }

    pub fn with_backoff(mut self, initial: Duration, max: Duration) -> Self {
        self.initial_backoff = initial;
        self.max_backoff = max;
        self
    }

    pub fn len(&self) -> usize {
        self.active.len() + self.backoff.len() + self.unschedulable.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn active_len(&self) -> usize {
        self.active.len()
    }
    pub fn backoff_len(&self) -> usize {
        self.backoff.len()
    }
    pub fn unschedulable_len(&self) -> usize {
        self.unschedulable.len()
    }

    pub fn add(&mut self, pod: Pod) {
        self.active.push(QueuedPod {
            pod,
            enqueued_at: Utc::now(),
            backoff_until: None,
            attempts: 0,
        });
    }

    /// Pop the highest-priority pod from `active`. Returns None if empty.
    pub fn pop(&mut self) -> Option<Pod> {
        self.active.pop().map(|q| q.pod)
    }

    /// Mark a pod as failed in a recoverable way → moves to backoff with exponential delay.
    /// Returns the new backoff_until.
    pub fn mark_backoff(&mut self, pod: Pod, now: DateTime<Utc>) -> DateTime<Utc> {
        self.unschedulable.remove(&pod.uid);
        let attempts = self
            .attempts_by_uid
            .entry(pod.uid.clone())
            .and_modify(|n| *n += 1)
            .or_insert(1)
            .clone();
        let exp = (self.initial_backoff * 2_i32.saturating_pow(attempts.saturating_sub(1).min(10)))
            .min(self.max_backoff);
        let until = now + exp;
        self.backoff.push(QueuedPod {
            pod,
            enqueued_at: now,
            backoff_until: Some(until),
            attempts,
        });
        until
    }

    /// Mark a pod as failed in an unresolvable way → moves to unschedulable until move().
    /// Preserves the failure counter (only mark_backoff increments it); a pod that
    /// flips between backoff and unschedulable counts each backoff as one failure.
    pub fn mark_unschedulable(&mut self, pod: Pod, now: DateTime<Utc>) {
        let attempts = self.attempts_by_uid.get(&pod.uid).copied().unwrap_or(0);
        self.unschedulable.insert(
            pod.uid.clone(),
            QueuedPod {
                pod,
                enqueued_at: now,
                backoff_until: None,
                attempts,
            },
        );
    }

    /// Flush expired backoff entries back to active.
    pub fn flush_backoff(&mut self, now: DateTime<Utc>) {
        let mut keep: Vec<QueuedPod> = vec![];
        for q in self.backoff.drain(..) {
            if q.backoff_until.map_or(true, |t| t <= now) {
                self.active.push(QueuedPod {
                    backoff_until: None,
                    ..q
                });
            } else {
                keep.push(q);
            }
        }
        self.backoff = keep;
    }

    /// Move all unschedulable pods back to active (e.g. after a cluster event).
    pub fn move_all_unschedulable(&mut self, now: DateTime<Utc>) {
        for (_, mut q) in self.unschedulable.drain() {
            q.enqueued_at = now;
            self.active.push(q);
        }
    }
}

impl Default for PriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(name: &str, prio: i32) -> Pod {
        let mut p = Pod::new("t", "ns", name);
        p.spec.priority = prio;
        p
    }

    #[test]
    fn pop_in_priority_order() {
        let mut q = PriorityQueue::new();
        q.add(pod("low", 1));
        q.add(pod("high", 100));
        q.add(pod("mid", 50));
        assert_eq!(q.pop().unwrap().name, "high");
        assert_eq!(q.pop().unwrap().name, "mid");
        assert_eq!(q.pop().unwrap().name, "low");
        assert!(q.pop().is_none());
    }

    #[test]
    fn fifo_within_same_priority() {
        let mut q = PriorityQueue::new();
        q.add(pod("a", 5));
        std::thread::sleep(std::time::Duration::from_millis(2));
        q.add(pod("b", 5));
        std::thread::sleep(std::time::Duration::from_millis(2));
        q.add(pod("c", 5));
        assert_eq!(q.pop().unwrap().name, "a");
        assert_eq!(q.pop().unwrap().name, "b");
        assert_eq!(q.pop().unwrap().name, "c");
    }

    #[test]
    fn backoff_attempts_persist_via_unschedulable() {
        let mut q = PriorityQueue::new().with_backoff(Duration::seconds(1), Duration::seconds(8));
        let now = Utc::now();
        let p = pod("p", 1);
        // First failure → backoff (attempts=1, 1s)
        let d1 = q.mark_backoff(p.clone(), now);
        assert_eq!((d1 - now).num_seconds(), 1);
        // Move to unschedulable holding attempts → mark_backoff reads from unschedulable
        q.mark_unschedulable(p.clone(), now);
        let d2 = q.mark_backoff(p.clone(), now);
        assert_eq!((d2 - now).num_seconds(), 2);
        q.mark_unschedulable(p.clone(), now);
        let d3 = q.mark_backoff(p.clone(), now);
        assert_eq!((d3 - now).num_seconds(), 4);
        q.mark_unschedulable(p.clone(), now);
        let d4 = q.mark_backoff(p.clone(), now);
        assert_eq!((d4 - now).num_seconds(), 8); // capped at max
        q.mark_unschedulable(p.clone(), now);
        let d5 = q.mark_backoff(p, now);
        assert_eq!((d5 - now).num_seconds(), 8); // still capped
    }

    #[test]
    fn flush_backoff_returns_to_active_after_deadline() {
        let mut q = PriorityQueue::new().with_backoff(Duration::seconds(1), Duration::seconds(10));
        let now = Utc::now();
        let until = q.mark_backoff(pod("p", 1), now);
        assert_eq!(q.backoff_len(), 1);
        assert_eq!(q.active_len(), 0);

        q.flush_backoff(now); // before deadline
        assert_eq!(q.backoff_len(), 1);
        assert_eq!(q.active_len(), 0);

        q.flush_backoff(until + Duration::milliseconds(1));
        assert_eq!(q.backoff_len(), 0);
        assert_eq!(q.active_len(), 1);
    }

    #[test]
    fn unschedulable_stays_until_move() {
        let mut q = PriorityQueue::new();
        let now = Utc::now();
        q.mark_unschedulable(pod("p1", 5), now);
        q.mark_unschedulable(pod("p2", 9), now);
        assert_eq!(q.unschedulable_len(), 2);
        assert_eq!(q.active_len(), 0);

        q.move_all_unschedulable(now);
        assert_eq!(q.unschedulable_len(), 0);
        assert_eq!(q.active_len(), 2);
        // Higher priority comes first.
        assert_eq!(q.pop().unwrap().name, "p2");
    }
}
