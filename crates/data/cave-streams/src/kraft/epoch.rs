// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Controller epoch + voter set — the two opaque values the
//! Raft layer feeds the controller on each leadership change.
//!
//! Mirrors `org.apache.kafka.raft.LeaderAndEpoch` from the
//! upstream `raft/` package.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Monotonic counter that increments every time leadership
/// transitions in the controller quorum. Every `MetadataRecord`
/// that lands in the log carries the epoch at the moment of
/// append — old leaders that try to replay records past their
/// epoch are rejected.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct ControllerEpoch(pub u64);

impl ControllerEpoch {
    /// The initial epoch when no leadership has been established.
    pub const INITIAL: Self = Self(0);

    /// Successor of `self` — used when a new election concludes.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// The static set of voter nodes participating in the metadata
/// quorum. Configured at controller startup, immutable for the
/// life of the controller (KRaft re-config is KIP-853, post-4.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoterSet {
    voters: BTreeSet<i32>,
    leader: Option<i32>,
    epoch: ControllerEpoch,
}

impl VoterSet {
    /// New voter set with no leader (pre-first-election state).
    pub fn new(voters: impl IntoIterator<Item = i32>) -> Self {
        Self {
            voters: voters.into_iter().collect(),
            leader: None,
            epoch: ControllerEpoch::INITIAL,
        }
    }

    /// Total voter count. Quorum is `floor(N/2) + 1`.
    pub fn size(&self) -> usize {
        self.voters.len()
    }

    /// Minimum size for an append to commit.
    pub fn quorum(&self) -> usize {
        self.voters.len() / 2 + 1
    }

    pub fn contains(&self, node_id: i32) -> bool {
        self.voters.contains(&node_id)
    }

    pub fn leader(&self) -> Option<i32> {
        self.leader
    }

    pub fn epoch(&self) -> ControllerEpoch {
        self.epoch
    }

    /// Record a new election outcome. Returns `Err` if `node_id`
    /// isn't in the voter set, or if `new_epoch` isn't strictly
    /// greater than the current epoch (stale leadership claim).
    pub fn elect(&mut self, node_id: i32, new_epoch: ControllerEpoch) -> Result<(), String> {
        if !self.voters.contains(&node_id) {
            return Err(format!("node {node_id} not in voter set"));
        }
        if new_epoch <= self.epoch {
            return Err(format!(
                "stale epoch {new_epoch:?}; current is {:?}",
                self.epoch
            ));
        }
        self.leader = Some(node_id);
        self.epoch = new_epoch;
        Ok(())
    }

    /// Step down — leader becomes `None`, epoch unchanged.
    /// Used when a follower observes a higher-epoch heartbeat.
    pub fn step_down(&mut self) {
        self.leader = None;
    }

    /// Add a single voter to the quorum (KIP-853 dynamic
    /// reconfiguration). Mirrors `AddRaftVoter` from upstream's
    /// `KafkaRaftClient`: a membership change is committed as its own
    /// quorum event, so the controller epoch advances to fence any
    /// node still operating against the pre-change voter set.
    ///
    /// Returns `Err` if `node_id` is already a voter, or if
    /// `new_epoch` does not strictly advance the current epoch (a
    /// stale or replayed reconfiguration). Per Raft single-server
    /// change rules only one voter is added at a time, so the new
    /// majority always overlaps the old majority — no split-brain
    /// window exists. Leadership is preserved across the change.
    pub fn add_voter(&mut self, node_id: i32, new_epoch: ControllerEpoch) -> Result<(), String> {
        if self.voters.contains(&node_id) {
            return Err(format!("node {node_id} is already a voter"));
        }
        if new_epoch <= self.epoch {
            return Err(format!(
                "stale reconfiguration epoch {new_epoch:?}; current is {:?}",
                self.epoch
            ));
        }
        self.voters.insert(node_id);
        self.epoch = new_epoch;
        Ok(())
    }

    /// Remove a single voter from the quorum (KIP-853). Mirrors
    /// `RemoveRaftVoter` from upstream. Advances the epoch like
    /// [`add_voter`](Self::add_voter).
    ///
    /// Returns `Err` if `node_id` is not a current voter, if the
    /// removal would empty the quorum (the metadata log would be
    /// lost), or on a stale epoch. If the removed node is the current
    /// leader, the set steps down — the surviving voters must elect a
    /// new leader at a higher epoch.
    pub fn remove_voter(&mut self, node_id: i32, new_epoch: ControllerEpoch) -> Result<(), String> {
        if !self.voters.contains(&node_id) {
            return Err(format!("node {node_id} is not a voter"));
        }
        if self.voters.len() == 1 {
            return Err("cannot remove the last voter from the quorum".into());
        }
        if new_epoch <= self.epoch {
            return Err(format!(
                "stale reconfiguration epoch {new_epoch:?}; current is {:?}",
                self.epoch
            ));
        }
        self.voters.remove(&node_id);
        if self.leader == Some(node_id) {
            self.leader = None;
        }
        self.epoch = new_epoch;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_starts_at_zero_and_advances() {
        let e0 = ControllerEpoch::INITIAL;
        assert_eq!(e0.0, 0);
        let e1 = e0.next();
        assert_eq!(e1.0, 1);
        assert!(e1 > e0);
    }

    #[test]
    fn quorum_is_majority() {
        assert_eq!(VoterSet::new([1]).quorum(), 1);
        assert_eq!(VoterSet::new([1, 2, 3]).quorum(), 2);
        assert_eq!(VoterSet::new([1, 2, 3, 4, 5]).quorum(), 3);
    }

    #[test]
    fn voter_set_contains_only_configured_nodes() {
        let v = VoterSet::new([1, 2, 3]);
        assert!(v.contains(2));
        assert!(!v.contains(4));
        assert_eq!(v.size(), 3);
    }

    #[test]
    fn elect_advances_leader_and_epoch() {
        let mut v = VoterSet::new([1, 2, 3]);
        v.elect(2, ControllerEpoch(1)).unwrap();
        assert_eq!(v.leader(), Some(2));
        assert_eq!(v.epoch(), ControllerEpoch(1));
    }

    #[test]
    fn elect_rejects_non_voter() {
        let mut v = VoterSet::new([1, 2, 3]);
        assert!(v.elect(42, ControllerEpoch(1)).is_err());
        assert!(v.leader().is_none());
    }

    #[test]
    fn elect_rejects_stale_epoch() {
        let mut v = VoterSet::new([1, 2, 3]);
        v.elect(1, ControllerEpoch(5)).unwrap();
        // Same epoch — stale.
        assert!(v.elect(2, ControllerEpoch(5)).is_err());
        // Lower epoch — also stale.
        assert!(v.elect(2, ControllerEpoch(3)).is_err());
        // Strictly higher — accepted.
        v.elect(2, ControllerEpoch(6)).unwrap();
        assert_eq!(v.leader(), Some(2));
    }

    #[test]
    fn step_down_clears_leader_keeps_epoch() {
        let mut v = VoterSet::new([1, 2, 3]);
        v.elect(2, ControllerEpoch(3)).unwrap();
        v.step_down();
        assert!(v.leader().is_none());
        assert_eq!(v.epoch(), ControllerEpoch(3));
    }

    // ── KIP-853: dynamic KRaft quorum reconfiguration ──────────────────────

    #[test]
    fn add_voter_grows_set_and_bumps_epoch() {
        let mut v = VoterSet::new([1, 2, 3]);
        v.elect(1, ControllerEpoch(4)).unwrap();
        v.add_voter(4, ControllerEpoch(5)).unwrap();
        assert!(v.contains(4));
        assert_eq!(v.size(), 4);
        // A reconfiguration is itself a quorum-committed event — the
        // epoch advances so stale leaders observing the old set fence out.
        assert_eq!(v.epoch(), ControllerEpoch(5));
        // Leadership is preserved across the membership change.
        assert_eq!(v.leader(), Some(1));
    }

    #[test]
    fn add_existing_voter_is_rejected() {
        let mut v = VoterSet::new([1, 2, 3]);
        assert!(v.add_voter(2, ControllerEpoch(5)).is_err());
        assert_eq!(v.size(), 3);
    }

    #[test]
    fn add_voter_rejects_stale_epoch() {
        let mut v = VoterSet::new([1, 2, 3]);
        v.elect(1, ControllerEpoch(5)).unwrap();
        assert!(v.add_voter(4, ControllerEpoch(5)).is_err());
        assert!(!v.contains(4));
    }

    #[test]
    fn add_voter_is_one_at_a_time_quorum_change() {
        // KIP-853 follows Raft single-server changes: the new quorum
        // (4 nodes, majority 3) overlaps the old quorum (3 nodes,
        // majority 2) so no split-brain window exists.
        let mut v = VoterSet::new([1, 2, 3]);
        assert_eq!(v.quorum(), 2);
        v.add_voter(4, ControllerEpoch(1)).unwrap();
        assert_eq!(v.quorum(), 3);
    }

    #[test]
    fn remove_voter_shrinks_set_and_bumps_epoch() {
        let mut v = VoterSet::new([1, 2, 3]);
        v.elect(1, ControllerEpoch(2)).unwrap();
        v.remove_voter(3, ControllerEpoch(3)).unwrap();
        assert!(!v.contains(3));
        assert_eq!(v.size(), 2);
        assert_eq!(v.epoch(), ControllerEpoch(3));
        assert_eq!(v.leader(), Some(1));
    }

    #[test]
    fn remove_unknown_voter_is_rejected() {
        let mut v = VoterSet::new([1, 2, 3]);
        assert!(v.remove_voter(9, ControllerEpoch(5)).is_err());
        assert_eq!(v.size(), 3);
    }

    #[test]
    fn remove_last_voter_is_rejected() {
        // A quorum cannot shrink to zero — the controller would lose
        // its metadata log forever. Upstream guards this too.
        let mut v = VoterSet::new([1]);
        assert!(v.remove_voter(1, ControllerEpoch(5)).is_err());
        assert_eq!(v.size(), 1);
    }

    #[test]
    fn remove_current_leader_steps_down() {
        let mut v = VoterSet::new([1, 2, 3]);
        v.elect(2, ControllerEpoch(2)).unwrap();
        v.remove_voter(2, ControllerEpoch(3)).unwrap();
        assert!(!v.contains(2));
        // The removed node was the leader — the set has no leader until
        // the surviving voters elect a new one.
        assert!(v.leader().is_none());
        assert_eq!(v.epoch(), ControllerEpoch(3));
    }
}
