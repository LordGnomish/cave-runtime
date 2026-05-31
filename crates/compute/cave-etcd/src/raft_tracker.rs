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

use std::collections::{BTreeMap, BTreeSet};

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

/// Replication state of one follower as seen by the leader.
///
/// Mirrors etcd `tracker.StateType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressState {
    /// At most one append in flight; the leader is searching for the follower's
    /// match index after (re)gaining leadership or a rejection.
    Probe,
    /// The follower's log is caught up enough to stream entries optimistically,
    /// bounded by the [`Inflights`] window.
    Replicate,
    /// A snapshot is being installed; no appends are sent until it completes.
    Snapshot,
}

/// Per-follower replication progress on the leader.
///
/// Mirrors etcd `tracker.Progress` (`raft/tracker/progress.go`). Tracks the
/// highest replicated index (`match`), the next index to send (`next`), the
/// replication [`ProgressState`], and the flow-control window.
#[derive(Clone, Debug)]
pub struct Progress {
    /// Highest log index known to be replicated on the follower.
    pub r#match: u64,
    /// Next log index the leader will send to the follower.
    pub next: u64,
    /// Current replication state.
    pub state: ProgressState,
    /// In `Snapshot` state, the snapshot index being installed.
    pub pending_snapshot: u64,
    /// Whether the follower has been heard from in the current election timeout.
    pub recent_active: bool,
    /// In `Probe` state, whether an append is already outstanding (pauses sends).
    /// (etcd v3.6 renamed `ProbeSent` → `MsgAppFlowPaused`.)
    pub msg_app_flow_paused: bool,
    /// Whether this peer is a non-voting learner.
    pub is_learner: bool,
    /// Sliding-window flow control for `Replicate` state.
    pub inflights: Inflights,
}

impl Progress {
    /// Create a fresh `Probe`-state progress starting at `next`, with an
    /// in-flight window of `inflight_size`.
    pub fn new(next: u64, inflight_size: usize) -> Self {
        Self {
            r#match: 0,
            next,
            state: ProgressState::Probe,
            pending_snapshot: 0,
            recent_active: false,
            msg_app_flow_paused: false,
            is_learner: false,
            inflights: Inflights::new(inflight_size),
        }
    }

    /// Reset the volatile flow-control fields when transitioning to `state`.
    fn reset_state(&mut self, state: ProgressState) {
        self.msg_app_flow_paused = false;
        self.pending_snapshot = 0;
        self.state = state;
        self.inflights.reset();
    }

    /// Mark that the in-flight probe has been acknowledged, resuming sends.
    pub fn probe_acked(&mut self) {
        self.msg_app_flow_paused = false;
    }

    /// Transition to `Probe`. `next` is set just past the better of the known
    /// match index and any pending snapshot index.
    pub fn become_probe(&mut self) {
        // If a snapshot was in flight, resume probing past it; otherwise probe
        // just past the known match index.
        if self.state == ProgressState::Snapshot {
            let pending = self.pending_snapshot;
            self.reset_state(ProgressState::Probe);
            self.next = (self.r#match + 1).max(pending + 1);
        } else {
            self.reset_state(ProgressState::Probe);
            self.next = self.r#match + 1;
        }
    }

    /// Transition to `Replicate`, streaming from `match + 1`.
    pub fn become_replicate(&mut self) {
        self.reset_state(ProgressState::Replicate);
        self.next = self.r#match + 1;
    }

    /// Transition to `Snapshot`, recording the snapshot index being installed.
    pub fn become_snapshot(&mut self, snapshot_idx: u64) {
        self.reset_state(ProgressState::Snapshot);
        self.pending_snapshot = snapshot_idx;
    }

    /// Apply an acknowledgement of index `n`. Returns whether `match` advanced.
    pub fn maybe_update(&mut self, n: u64) -> bool {
        let mut updated = false;
        if self.r#match < n {
            self.r#match = n;
            updated = true;
            self.probe_acked();
        }
        self.next = self.next.max(n + 1);
        updated
    }

    /// Optimistically advance `next` after sending entries up to `n` (Replicate).
    pub fn optimistic_update(&mut self, n: u64) {
        self.next = n + 1;
    }

    /// React to an `AppendEntries` rejection reported by the follower.
    ///
    /// `rejected` is the index the follower refused; `match_hint` is the
    /// follower's last matching index hint. Returns whether `next` was rolled
    /// back (a stale/spurious rejection returns `false` and changes nothing).
    pub fn maybe_decr_to(&mut self, rejected: u64, match_hint: u64) -> bool {
        if self.state == ProgressState::Replicate {
            // We optimistically stream in Replicate, so a rejection at or below
            // the known match index is obsolete and must be ignored.
            if rejected <= self.r#match {
                return false;
            }
            self.next = self.r#match + 1;
            return true;
        }
        // Probe/Snapshot: the rejection must concern the single in-flight probe
        // at `next - 1`; anything else is stale.
        if self.next == 0 || self.next - 1 != rejected {
            return false;
        }
        self.next = rejected.min(match_hint + 1).max(1);
        self.msg_app_flow_paused = false;
        true
    }

    /// Whether the leader must currently withhold appends to this follower.
    pub fn is_paused(&self) -> bool {
        match self.state {
            ProgressState::Probe => self.msg_app_flow_paused,
            ProgressState::Replicate => self.inflights.full(),
            ProgressState::Snapshot => true,
        }
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn become_replicate_streams_from_match_plus_one() {
        let mut pr = Progress::new(1, 4);
        pr.r#match = 5;
        pr.become_replicate();
        assert_eq!(pr.state, ProgressState::Replicate);
        assert_eq!(pr.next, 6);
    }

    #[test]
    fn become_probe_from_replicate_uses_match() {
        let mut pr = Progress::new(1, 4);
        pr.r#match = 5;
        pr.become_replicate();
        pr.become_probe();
        assert_eq!(pr.state, ProgressState::Probe);
        assert_eq!(pr.next, 6); // match + 1
    }

    #[test]
    fn become_probe_from_snapshot_uses_pending_snapshot_floor() {
        let mut pr = Progress::new(1, 4);
        pr.r#match = 3;
        pr.become_snapshot(11);
        assert_eq!(pr.state, ProgressState::Snapshot);
        assert_eq!(pr.pending_snapshot, 11);
        pr.become_probe();
        assert_eq!(pr.state, ProgressState::Probe);
        // max(match+1=4, pending_snapshot+1=12) = 12
        assert_eq!(pr.next, 12);
        assert_eq!(pr.pending_snapshot, 0, "pending snapshot cleared on reset");
    }

    #[test]
    fn maybe_update_advances_and_reports() {
        let mut pr = Progress::new(1, 4);
        assert!(pr.maybe_update(5), "first ack advances match");
        assert_eq!(pr.r#match, 5);
        assert_eq!(pr.next, 6);
        assert!(!pr.maybe_update(3), "stale ack does not advance match");
        assert_eq!(pr.r#match, 5);
        assert_eq!(pr.next, 6, "next never regresses");
    }

    #[test]
    fn optimistic_update_sets_next() {
        let mut pr = Progress::new(1, 4);
        pr.optimistic_update(9);
        assert_eq!(pr.next, 10);
    }

    #[test]
    fn maybe_decr_to_in_replicate() {
        let mut pr = Progress::new(1, 4);
        pr.r#match = 5;
        pr.become_replicate(); // next = 6
        // Rejection at or below match is obsolete → ignored.
        assert!(!pr.maybe_decr_to(5, 5));
        assert_eq!(pr.next, 6);
        // Genuine rejection above match → snap next back to match+1.
        pr.next = 12;
        assert!(pr.maybe_decr_to(11, 7));
        assert_eq!(pr.next, 6);
    }

    #[test]
    fn maybe_decr_to_in_probe_respects_hint_and_floor() {
        let mut pr = Progress::new(10, 4); // Probe, next = 10
        // Stale rejection (not for the in-flight probe at next-1=9) → no-op.
        assert!(!pr.maybe_decr_to(7, 7));
        assert_eq!(pr.next, 10);
        // Valid rejection for next-1=9; hint says follower has up to 4.
        pr.msg_app_flow_paused = true;
        assert!(pr.maybe_decr_to(9, 4));
        // next = max(min(rejected=9, hint+1=5), 1) = 5
        assert_eq!(pr.next, 5);
        assert!(!pr.msg_app_flow_paused, "rejection resumes probing");
    }

    #[test]
    fn maybe_decr_to_probe_floor_is_one() {
        let mut pr = Progress::new(1, 4); // next = 1, next-1 = 0
        assert!(pr.maybe_decr_to(0, 0));
        assert_eq!(pr.next, 1, "next never drops below 1");
    }

    #[test]
    fn is_paused_per_state() {
        // Probe: paused iff an append is outstanding.
        let mut pr = Progress::new(1, 2);
        assert!(!pr.is_paused());
        pr.msg_app_flow_paused = true;
        assert!(pr.is_paused());

        // Replicate: paused iff the inflight window is full.
        pr.r#match = 1;
        pr.become_replicate();
        assert!(!pr.is_paused());
        pr.inflights.add(2);
        pr.inflights.add(3);
        assert!(pr.is_paused(), "full window pauses replicate");

        // Snapshot: always paused.
        pr.become_snapshot(9);
        assert!(pr.is_paused());
    }
}

/// Outcome of counting election votes against the voter configuration.
///
/// Mirrors etcd `quorum.VoteResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoteResult {
    /// Not enough votes counted yet to decide either way.
    Pending,
    /// A quorum has rejected the candidate.
    Lost,
    /// A quorum has granted the candidate.
    Won,
}

/// Resolve a vote tally against a single majority configuration.
///
/// Mirrors etcd `quorum.MajorityConfig.VoteResult`. An empty configuration is
/// vacuously [`Won`](VoteResult::Won).
fn majority_vote_result(voters: &BTreeSet<u64>, votes: &BTreeMap<u64, bool>) -> VoteResult {
    if voters.is_empty() {
        return VoteResult::Won;
    }
    let quorum = voters.len() / 2 + 1;
    let mut granted = 0usize;
    let mut missing = 0usize;
    for v in voters {
        match votes.get(v) {
            Some(true) => granted += 1,
            Some(false) => {}
            None => missing += 1,
        }
    }
    if granted >= quorum {
        VoteResult::Won
    } else if granted + missing >= quorum {
        VoteResult::Pending
    } else {
        VoteResult::Lost
    }
}

/// The leader-side replication tracker: per-follower [`Progress`] plus the
/// campaign vote ledger, resolved against a (possibly joint) voter config.
///
/// Mirrors etcd `tracker.ProgressTracker` (`raft/tracker/tracker.go`).
#[derive(Clone, Debug)]
pub struct ProgressTracker {
    /// Incoming voter configuration (C_new during a joint change).
    incoming: BTreeSet<u64>,
    /// Outgoing voter configuration (C_old; empty when not in a joint change).
    outgoing: BTreeSet<u64>,
    /// Non-voting learners.
    learners: BTreeSet<u64>,
    /// Per-peer replication progress.
    pub progress: BTreeMap<u64, Progress>,
    /// Recorded campaign votes (`true` = granted), first write wins.
    votes: BTreeMap<u64, bool>,
    /// Flow-control window size for newly tracked peers.
    max_inflight: usize,
}

impl ProgressTracker {
    /// Create an empty tracker whose followers get an `max_inflight`-deep window.
    pub fn new(max_inflight: usize) -> Self {
        Self {
            incoming: BTreeSet::new(),
            outgoing: BTreeSet::new(),
            learners: BTreeSet::new(),
            progress: BTreeMap::new(),
            votes: BTreeMap::new(),
            max_inflight,
        }
    }

    /// Add a voting member, creating its [`Progress`].
    pub fn add_voter(&mut self, id: u64) {
        self.incoming.insert(id);
        self.progress
            .entry(id)
            .or_insert_with(|| Progress::new(1, self.max_inflight));
    }

    /// Add a non-voting learner, creating its [`Progress`].
    pub fn add_learner(&mut self, id: u64) {
        self.learners.insert(id);
        let mut pr = Progress::new(1, self.max_inflight);
        pr.is_learner = true;
        self.progress.insert(id, pr);
    }

    /// Enter a joint configuration: the current voters become `C_old` and
    /// `incoming` becomes the proposed `C_new`.
    pub fn make_joint(&mut self, outgoing: &[u64]) {
        self.outgoing = outgoing.iter().copied().collect();
    }

    /// Record a vote from `id`. The first vote recorded for a member wins;
    /// later votes for the same member are ignored (matching etcd).
    pub fn record_vote(&mut self, id: u64, granted: bool) {
        self.votes.entry(id).or_insert(granted);
    }

    /// Clear all recorded votes (etcd `ProgressTracker.ResetVotes`), called
    /// when a fresh campaign begins so stale grants from a prior term do not
    /// leak into the new tally.
    pub fn reset_votes(&mut self) {
        self.votes.clear();
    }

    /// The incoming (`C_new`) voter set.
    pub fn voter_ids(&self) -> Vec<u64> {
        self.incoming.iter().copied().collect()
    }

    /// The outgoing (`C_old`) voter set — empty when not in a joint change.
    pub fn outgoing_ids(&self) -> Vec<u64> {
        self.outgoing.iter().copied().collect()
    }

    /// Count votes among voters, excluding learners. Returns
    /// `(granted, rejected)`.
    pub fn tally_votes(&self) -> (usize, usize) {
        let mut granted = 0usize;
        let mut rejected = 0usize;
        // Only members that hold (non-learner) progress count toward the tally.
        for (id, pr) in &self.progress {
            if pr.is_learner {
                continue;
            }
            match self.votes.get(id) {
                Some(true) => granted += 1,
                Some(false) => rejected += 1,
                None => {}
            }
        }
        (granted, rejected)
    }

    /// Resolve the recorded votes against the joint voter configuration.
    ///
    /// The result is [`Won`](VoteResult::Won) only if BOTH the incoming and
    /// outgoing majority configs are won; [`Lost`](VoteResult::Lost) if either
    /// is lost; otherwise [`Pending`](VoteResult::Pending).
    pub fn vote_result(&self) -> VoteResult {
        let r_in = majority_vote_result(&self.incoming, &self.votes);
        let r_out = majority_vote_result(&self.outgoing, &self.votes);
        if r_in == r_out {
            return r_in;
        }
        if r_in == VoteResult::Lost || r_out == VoteResult::Lost {
            return VoteResult::Lost;
        }
        VoteResult::Pending
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;

    fn voters(t: &mut ProgressTracker, ids: &[u64]) {
        for &id in ids {
            t.add_voter(id);
        }
    }

    #[test]
    fn single_config_majority_grant_wins() {
        let mut t = ProgressTracker::new(4);
        voters(&mut t, &[1, 2, 3]);
        t.record_vote(1, true);
        assert_eq!(t.vote_result(), VoteResult::Pending);
        t.record_vote(2, true);
        assert_eq!(t.vote_result(), VoteResult::Won);
    }

    #[test]
    fn single_config_majority_reject_loses() {
        let mut t = ProgressTracker::new(4);
        voters(&mut t, &[1, 2, 3]);
        t.record_vote(1, false);
        t.record_vote(2, false);
        assert_eq!(t.vote_result(), VoteResult::Lost);
    }

    #[test]
    fn record_vote_first_write_wins() {
        let mut t = ProgressTracker::new(4);
        voters(&mut t, &[1, 2, 3]);
        t.record_vote(1, true);
        t.record_vote(1, false); // ignored
        let (granted, rejected) = t.tally_votes();
        assert_eq!((granted, rejected), (1, 0));
    }

    #[test]
    fn tally_excludes_learners() {
        let mut t = ProgressTracker::new(4);
        voters(&mut t, &[1, 2, 3]);
        t.add_learner(9);
        t.record_vote(1, true);
        t.record_vote(9, true); // learner vote does not count toward tally
        let (granted, rejected) = t.tally_votes();
        assert_eq!((granted, rejected), (1, 0));
        // And a learner can never push a 3-voter config to Won on its own.
        t.record_vote(9, true);
        assert_eq!(t.vote_result(), VoteResult::Pending);
    }

    #[test]
    fn joint_config_requires_both_majorities() {
        // C_old = {1,2,3}, C_new = {3,4,5}. A candidate must win BOTH.
        let mut t = ProgressTracker::new(4);
        voters(&mut t, &[3, 4, 5]); // incoming = C_new
        t.make_joint(&[1, 2, 3]); // outgoing = C_old
        // Win C_old (1,2) but only 1 vote in C_new → Pending overall.
        t.record_vote(1, true);
        t.record_vote(2, true);
        t.record_vote(3, true); // shared member, counts in both
        assert_eq!(t.vote_result(), VoteResult::Pending);
        // Add a second C_new vote → both majorities satisfied → Won.
        t.record_vote(4, true);
        assert_eq!(t.vote_result(), VoteResult::Won);
    }

    #[test]
    fn joint_config_lost_in_one_is_lost() {
        let mut t = ProgressTracker::new(4);
        voters(&mut t, &[3, 4, 5]);
        t.make_joint(&[1, 2, 3]);
        // Rejected by a majority of C_old → Lost regardless of C_new.
        t.record_vote(1, false);
        t.record_vote(2, false);
        t.record_vote(4, true);
        t.record_vote(5, true);
        assert_eq!(t.vote_result(), VoteResult::Lost);
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
