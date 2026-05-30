// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-process Raft progress tracking — the leader's per-follower replication
//! bookkeeping, message-flow control, and election vote tally.
//!
//! Ports etcd v3.6 `raft/tracker/`:
//!
//! * [`Inflights`] — `raft/tracker/inflights.go`. A fixed-capacity sliding
//!   window of in-flight `MsgApp` indices that bounds how far ahead of the
//!   acknowledged log a leader may optimistically stream entries to one
//!   follower.
//! * [`Progress`] — `raft/tracker/progress.go`. The replication state machine
//!   (`Probe`/`Replicate`/`Snapshot`) plus `match`/`next` indices for a single
//!   follower.
//! * [`ProgressTracker`] — `raft/tracker/tracker.go`. The leader-side map of
//!   follower [`Progress`] plus the campaign vote ledger, whose tally is
//!   resolved against the (possibly joint) voter configuration.
//!
//! This is **in-process data-plane logic**, distinct from the multi-node Raft
//! *transport* (`rafthttp`, tracked in cave-runtime cluster plumbing). It is
//! the natural sibling of the already-ported `raft/confchange/` (joint
//! membership shapes in `store.rs`) and `raft/quorum/` ([`crate::raft_joint_quorum`]).
//!
//! # References
//!
//! * <https://github.com/etcd-io/etcd/blob/v3.6.10/raft/tracker/inflights.go>
//! * <https://github.com/etcd-io/etcd/blob/v3.6.10/raft/tracker/progress.go>
//! * <https://github.com/etcd-io/etcd/blob/v3.6.10/raft/tracker/tracker.go>

#![allow(clippy::needless_range_loop)]

/// A sliding window of in-flight `MsgApp` message indices.
///
/// Mirrors etcd `tracker.Inflights`. The window holds at most `size` entries.
/// Indices are added in monotonically increasing order as the leader sends
/// append messages, and freed (in order) as the follower acknowledges them.
/// While [`full`](Inflights::full) is true the leader must stop sending new
/// appends to that follower.
///
/// Internally a ring buffer: `buffer[start ..][.. count]` (mod `size`) holds
/// the live indices.
#[derive(Clone, Debug)]
pub struct Inflights {
    /// Index of the oldest in-flight entry within `buffer`.
    start: usize,
    /// Number of in-flight entries.
    count: usize,
    /// Maximum number of in-flight entries (the window cap).
    size: usize,
    /// Ring buffer of log indices; grown lazily up to `size`.
    buffer: Vec<u64>,
}

impl Inflights {
    /// Create a window holding at most `size` in-flight messages.
    pub fn new(size: usize) -> Self {
        Self {
            start: 0,
            count: 0,
            size,
            buffer: Vec::new(),
        }
    }

    /// Record a new in-flight message whose last entry has index `inflight`.
    ///
    /// # Panics
    /// Panics if the window is already [`full`](Inflights::full) — callers must
    /// gate on `full()` first, exactly as etcd does.
    pub fn add(&mut self, inflight: u64) {
        if self.full() {
            panic!("cannot add into a full inflights window");
        }
        let mut next = self.start + self.count;
        if next >= self.size {
            next -= self.size;
        }
        // Grow the buffer lazily up to `size` — etcd amortises allocation the
        // same way rather than pre-sizing the (possibly large) window.
        if next >= self.buffer.len() {
            self.buffer.resize(next + 1, 0);
        }
        self.buffer[next] = inflight;
        self.count += 1;
    }

    /// Free all in-flight entries with index `<= to` (an acknowledgement).
    pub fn free_le(&mut self, to: u64) {
        if self.count == 0 || to < self.buffer[self.start] {
            // Out-of-order or duplicate ack covering nothing live.
            return;
        }
        let mut i = 0;
        let mut idx = self.start;
        while i < self.count {
            if to < self.buffer[idx] {
                // Reached an entry not yet acknowledged.
                break;
            }
            i += 1;
            idx += 1;
            if idx >= self.size {
                idx -= self.size;
            }
        }
        self.count -= i;
        self.start = idx;
        if self.count == 0 {
            // Reset to the head so the buffer reuses cleanly without wrapping.
            self.start = 0;
        }
    }

    /// Free the first (oldest) in-flight entry — used when a single message is
    /// known to have been received.
    pub fn free_first_one(&mut self) {
        if self.count == 0 {
            return;
        }
        self.free_le(self.buffer[self.start]);
    }

    /// True when no further messages may be sent (window saturated).
    pub fn full(&self) -> bool {
        self.count == self.size
    }

    /// Number of in-flight messages currently tracked.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Drop all in-flight entries (e.g. on a follower reset).
    pub fn reset(&mut self) {
        self.count = 0;
        self.start = 0;
    }
}

#[cfg(test)]
mod inflights_tests {
    use super::*;

    #[test]
    fn add_fills_then_full() {
        let mut ins = Inflights::new(4);
        assert!(!ins.full());
        for i in 0..4 {
            assert!(!ins.full(), "should not be full before {i} adds");
            ins.add(i);
        }
        assert!(ins.full(), "window of 4 must be full after 4 adds");
        assert_eq!(ins.count(), 4);
    }

    #[test]
    fn free_le_frees_acknowledged_prefix() {
        let mut ins = Inflights::new(8);
        for i in 1..=8 {
            ins.add(i);
        }
        assert!(ins.full());
        // Acknowledge through index 4 — frees the first 4.
        ins.free_le(4);
        assert_eq!(ins.count(), 4);
        assert!(!ins.full());
        // Acknowledging a lower index is a no-op (idempotent / out of order).
        ins.free_le(2);
        assert_eq!(ins.count(), 4);
        // Acknowledge the rest.
        ins.free_le(8);
        assert_eq!(ins.count(), 0);
    }

    #[test]
    fn free_first_one_frees_only_oldest() {
        let mut ins = Inflights::new(4);
        for i in 10..14 {
            ins.add(i);
        }
        assert!(ins.full());
        ins.free_first_one();
        assert_eq!(ins.count(), 3);
        assert!(!ins.full());
    }

    #[test]
    fn ring_buffer_wraps_around() {
        // Add, free, and re-add past the physical end of the buffer so the
        // window wraps. The live set must stay correct across the wrap.
        let mut ins = Inflights::new(4);
        for i in 0..4 {
            ins.add(i);
        }
        ins.free_le(1); // frees 0,1 → start advances to physical idx 2
        assert_eq!(ins.count(), 2);
        // Two more adds wrap into physical slots 0,1.
        ins.add(4);
        ins.add(5);
        assert!(ins.full());
        assert_eq!(ins.count(), 4);
        // Now free everything up to 5.
        ins.free_le(5);
        assert_eq!(ins.count(), 0);
        // start must reset to 0 when empty so the buffer can be reused cleanly.
        ins.add(6);
        assert_eq!(ins.count(), 1);
    }

    #[test]
    fn reset_clears_window() {
        let mut ins = Inflights::new(4);
        ins.add(1);
        ins.add(2);
        ins.reset();
        assert_eq!(ins.count(), 0);
        assert!(!ins.full());
    }

    #[test]
    #[should_panic]
    fn add_when_full_panics() {
        let mut ins = Inflights::new(2);
        ins.add(1);
        ins.add(2);
        ins.add(3); // overflow → panic, matching etcd's invariant guard
    }
}
