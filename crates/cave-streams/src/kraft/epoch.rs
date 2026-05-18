// SPDX-License-Identifier: AGPL-3.0-or-later
//! Controller epoch + voter set — the two opaque values the
//! Raft layer feeds the controller on each leadership change.
//!
//! Mirrors `org.apache.kafka.raft.LeaderAndEpoch` from the
//! upstream `raft/` package.

use std::collections::BTreeSet;

/// Monotonic counter that increments every time leadership
/// transitions in the controller quorum. Every `MetadataRecord`
/// that lands in the log carries the epoch at the moment of
/// append — old leaders that try to replay records past their
/// epoch are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
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
    pub fn elect(
        &mut self,
        node_id: i32,
        new_epoch: ControllerEpoch,
    ) -> Result<(), String> {
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
}
