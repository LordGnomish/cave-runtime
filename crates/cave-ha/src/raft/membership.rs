// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Joint-consensus membership change helpers.
//!
//! Raft membership changes use the two-phase joint-consensus protocol:
//! Phase 1 — Leader appends C_old,new (joint config). During this phase
//!            commits require quorum from BOTH the old and new voter sets.
//! Phase 2 — Once C_old,new is committed, leader appends C_new.
//!            From this point the new config is authoritative.

use crate::raft::types::{MembershipConfig, NodeId};
use std::collections::BTreeSet;

/// Compute the joint configuration for adding a node.
pub fn joint_for_add(
    current: &MembershipConfig,
    new_id: NodeId,
    is_learner: bool,
) -> MembershipConfig {
    let mut new_voters = current.voters.clone();
    let mut new_learners = current.learners.clone();
    if is_learner {
        new_learners.insert(new_id);
    } else {
        new_learners.remove(&new_id);
        new_voters.insert(new_id);
    }
    MembershipConfig {
        voters: new_voters,
        learners: new_learners,
        voters_outgoing: Some(current.voters.clone()),
        auto_leave: true,
    }
}

/// Compute the joint configuration for removing a node.
pub fn joint_for_remove(current: &MembershipConfig, remove_id: NodeId) -> MembershipConfig {
    let mut new_voters = current.voters.clone();
    new_voters.remove(&remove_id);
    let mut new_learners = current.learners.clone();
    new_learners.remove(&remove_id);
    MembershipConfig {
        voters: new_voters,
        learners: new_learners,
        voters_outgoing: Some(current.voters.clone()),
        auto_leave: true,
    }
}

/// Extract the C_new phase from a joint config (strip the outgoing set).
pub fn leave_joint(joint: &MembershipConfig) -> MembershipConfig {
    MembershipConfig {
        voters: joint.voters.clone(),
        learners: joint.learners.clone(),
        voters_outgoing: None,
        auto_leave: false,
    }
}

/// Validate a proposed membership change:
/// - Cannot remove last voter.
/// - Cannot have duplicate voter/learner.
pub fn validate(current: &MembershipConfig, proposed: &MembershipConfig) -> Result<(), String> {
    if proposed.voters.is_empty() {
        return Err("proposed config has no voters".into());
    }
    let overlap: BTreeSet<_> = proposed.voters.intersection(&proposed.learners).collect();
    if !overlap.is_empty() {
        return Err(format!(
            "nodes {overlap:?} appear in both voters and learners"
        ));
    }
    // Warn if removing more than one node at a time (not strictly invalid but risky).
    let removed: BTreeSet<_> = current.voters.difference(&proposed.voters).collect();
    if removed.len() > 1 {
        return Err("cannot remove more than one voter at a time".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(voters: &[u64]) -> MembershipConfig {
        MembershipConfig {
            voters: voters.iter().copied().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_quorum_sizes() {
        assert_eq!(MembershipConfig::quorum(1), 1);
        assert_eq!(MembershipConfig::quorum(2), 2);
        assert_eq!(MembershipConfig::quorum(3), 2);
        assert_eq!(MembershipConfig::quorum(5), 3);
        assert_eq!(MembershipConfig::quorum(7), 4);
    }

    #[test]
    fn test_joint_quorum() {
        let joint = MembershipConfig {
            voters: [3, 4, 5].iter().copied().collect(),
            voters_outgoing: Some([1, 2, 3].iter().copied().collect()),
            ..Default::default()
        };
        // Quorum from both sets required.
        let votes_ab: BTreeSet<u64> = [1, 3, 4].iter().copied().collect();
        assert!(joint.has_quorum(&votes_ab)); // 2/3 old, 2/3 new ✓

        let only_new: BTreeSet<u64> = [3, 4, 5].iter().copied().collect();
        assert!(!only_new.is_superset(&[1u64, 2].iter().copied().collect::<BTreeSet<_>>()));
        // Only new quorum without old quorum → fails.
        assert!(!joint.has_quorum(&[3, 4, 5].iter().copied().collect()));
    }

    #[test]
    fn test_joint_for_add() {
        let current = cfg(&[1, 2, 3]);
        let joint = joint_for_add(&current, 4, false);
        assert!(joint.voters.contains(&4));
        assert!(joint.voters_outgoing.as_ref().unwrap().contains(&1));
        assert!(joint.auto_leave);
    }

    #[test]
    fn test_validate_empty() {
        let current = cfg(&[1]);
        let empty = MembershipConfig::default();
        assert!(validate(&current, &empty).is_err());
    }
}
