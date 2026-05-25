// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Provisioning batcher — pending-pod queue with scheduling-round
//! semantics.
//!
//! Upstream reference (Karpenter v1.4.0):
//!   pkg/controllers/provisioning/batcher/batcher.go
//!
//! The upstream batcher accumulates pending pods over a short window
//! (default 1s, max 10s) so the scheduler can amortise NodePool decisions
//! across multiple pods rather than reacting one-by-one. The Cave port
//! preserves the window semantics, idle-debounce, and dedup by pod name.

use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Minimal pod-scheduling spec carried through the batcher into binpack.
#[derive(Debug, Clone, PartialEq)]
pub struct PodSpec {
    pub name: String,
    pub cpu_millis: u32,
    pub memory_mib: u32,
    /// If set, the pod requests topology-spread across the named
    /// label key (e.g. `topology.kubernetes.io/zone`).
    pub zone_spread_label: Option<String>,
    /// Tolerated taints — pods only schedule on instances whose taints
    /// are a subset of this set.
    pub tolerations: Vec<String>,
}

impl PodSpec {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            cpu_millis: 100,
            memory_mib: 128,
            zone_spread_label: None,
            tolerations: Vec::new(),
        }
    }

    pub fn with_resources(name: &str, cpu_millis: u32, memory_mib: u32) -> Self {
        Self {
            name: name.to_string(),
            cpu_millis,
            memory_mib,
            zone_spread_label: None,
            tolerations: Vec::new(),
        }
    }

    pub fn with_zone_spread(mut self, key: &str) -> Self {
        self.zone_spread_label = Some(key.to_string());
        self
    }

    pub fn tolerate(mut self, taint_key: &str) -> Self {
        self.tolerations.push(taint_key.to_string());
        self
    }
}

/// Pending-pod batcher. Construct with a window duration; pods enqueued
/// inside the window batch into a single scheduling round.
#[derive(Debug)]
pub struct Batcher {
    pending: Vec<PodSpec>,
    seen: HashSet<String>,
    window: Duration,
    window_start: Option<Instant>,
}

impl Batcher {
    pub fn new(window: Duration) -> Self {
        Self {
            pending: Vec::new(),
            seen: HashSet::new(),
            window,
            window_start: None,
        }
    }

    /// Idle-debounce window — once the first pod arrives, the batcher
    /// accumulates pods until `window` has elapsed, then [`take_round`]
    /// returns the accumulated set.
    pub fn window(&self) -> Duration {
        self.window
    }

    /// Enqueue a pod for the next round. Repeated enqueues for the same
    /// pod name in a single round are deduped — upstream behaviour for
    /// the case where the same pod's Reconcile fires twice before the
    /// batch flushes.
    pub fn enqueue(&mut self, pod: PodSpec) {
        if self.seen.insert(pod.name.clone()) {
            if self.window_start.is_none() {
                self.window_start = Some(Instant::now());
            }
            self.pending.push(pod);
        }
    }

    /// True once `window` has elapsed since the first enqueue.
    pub fn is_ready(&self, now: Instant) -> bool {
        match self.window_start {
            Some(start) => now.duration_since(start) >= self.window,
            None => false,
        }
    }

    /// Drain the pending pods into the caller — restarting the round.
    pub fn take_round(&mut self) -> Vec<PodSpec> {
        self.window_start = None;
        self.seen.clear();
        std::mem::take(&mut self.pending)
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batcher_window_starts_with_first_enqueue() {
        let mut b = Batcher::new(Duration::from_millis(10));
        assert!(!b.is_ready(Instant::now()));
        b.enqueue(PodSpec::new("p1"));
        std::thread::sleep(Duration::from_millis(15));
        assert!(b.is_ready(Instant::now()));
    }

    #[test]
    fn take_round_resets_window() {
        let mut b = Batcher::new(Duration::from_millis(5));
        b.enqueue(PodSpec::new("p1"));
        std::thread::sleep(Duration::from_millis(10));
        assert!(b.is_ready(Instant::now()));
        let _ = b.take_round();
        assert!(!b.is_ready(Instant::now()));
    }

    #[test]
    fn enqueue_after_take_starts_new_window() {
        let mut b = Batcher::new(Duration::from_millis(50));
        b.enqueue(PodSpec::new("p1"));
        let _ = b.take_round();
        b.enqueue(PodSpec::new("p2"));
        assert_eq!(b.pending_len(), 1);
    }
}
