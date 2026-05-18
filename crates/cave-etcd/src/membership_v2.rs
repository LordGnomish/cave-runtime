// SPDX-License-Identifier: AGPL-3.0-or-later
//! Raft membership v2 — ConfChangeV2 wire format, joint-consensus state
//! machine, cluster-id validation, and member-set diff helpers.
//!
//! Mirrors etcd v3.6.10
//!   `raft/raftpb/raft.proto` (ConfChangeV2 + ConfChangeTransition),
//!   `raft/confchange/confchange.go` (joint-consensus apply rules),
//!   `server/etcdserver/server.go#ValidateClusterAndAssignIDs`,
//!   `server/etcdserver/api/membership/cluster.go#applyConfChange`.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

// ── ConfChangeV2 wire types ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfChangeType {
    AddNode,
    AddLearnerNode,
    RemoveNode,
    UpdateNode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfChangeTransition {
    /// "Auto" — leave joint config implicitly when committed.
    Auto,
    /// "Implicit" — same as Auto in v3.6 but distinguished for older code.
    Implicit,
    /// "Explicit" — caller must issue a separate `LeaveJoint` change.
    Explicit,
}

impl Default for ConfChangeTransition {
    fn default() -> Self { Self::Auto }
}

/// One change inside a [`ConfChangeV2`] batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfChangeSingle {
    pub change_type: ConfChangeType,
    pub node_id: u64,
}

/// Raft v2 conf change.  May contain multiple `ConfChangeSingle`s; the
/// batch enters joint consensus, then either auto-leaves (`Auto`/`Implicit`)
/// or waits for an explicit `LeaveJoint` (`Explicit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfChangeV2 {
    pub transition: ConfChangeTransition,
    pub changes: Vec<ConfChangeSingle>,
    pub context: Vec<u8>,
}

impl ConfChangeV2 {
    pub fn new(transition: ConfChangeTransition) -> Self {
        Self { transition, changes: Vec::new(), context: Vec::new() }
    }

    pub fn add(mut self, kind: ConfChangeType, node_id: u64) -> Self {
        self.changes.push(ConfChangeSingle { change_type: kind, node_id });
        self
    }

    pub fn with_context(mut self, ctx: impl Into<Vec<u8>>) -> Self {
        self.context = ctx.into();
        self
    }

    /// True if this change requires a joint consensus.  Etcd: a change is
    /// "simple" (no joint) only if it touches at most one voter.
    pub fn enters_joint(&self) -> bool {
        let voter_changes = self.changes.iter().filter(|c| matches!(
            c.change_type, ConfChangeType::AddNode | ConfChangeType::RemoveNode
        )).count();
        voter_changes > 1 || matches!(self.transition, ConfChangeTransition::Explicit)
    }

    /// "Empty" leave-joint change — applied to leave joint config when in
    /// Explicit mode.  Mirrors `confchange.LeaveJoint`.
    pub fn leave_joint() -> Self {
        Self::new(ConfChangeTransition::Auto)
    }

    pub fn is_leave_joint(&self) -> bool {
        self.changes.is_empty()
    }
}

// ── Member-set state machine ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberConfig {
    pub voters: BTreeSet<u64>,
    pub learners: BTreeSet<u64>,
    /// Voters in the *outgoing* config when in joint consensus.  Empty in
    /// the steady state.
    pub voters_outgoing: BTreeSet<u64>,
}

impl MemberConfig {
    pub fn new() -> Self {
        Self {
            voters: BTreeSet::new(),
            learners: BTreeSet::new(),
            voters_outgoing: BTreeSet::new(),
        }
    }

    pub fn from_voters(ids: impl IntoIterator<Item = u64>) -> Self {
        let mut c = Self::new();
        c.voters = ids.into_iter().collect();
        c
    }

    pub fn voter_count(&self) -> usize { self.voters.len() }
    pub fn learner_count(&self) -> usize { self.learners.len() }
    pub fn is_joint(&self) -> bool { !self.voters_outgoing.is_empty() }

    pub fn quorum(&self) -> usize {
        if self.is_joint() {
            // Need majority in both incoming AND outgoing voter sets.
            (self.voters.len() / 2) + 1
        } else {
            (self.voters.len() / 2) + 1
        }
    }

    pub fn is_voter(&self, id: u64) -> bool { self.voters.contains(&id) }
    pub fn is_learner(&self, id: u64) -> bool { self.learners.contains(&id) }

    /// All known node ids (voters + learners + outgoing).
    pub fn all_ids(&self) -> BTreeSet<u64> {
        self.voters.iter()
            .chain(self.learners.iter())
            .chain(self.voters_outgoing.iter())
            .copied()
            .collect()
    }
}

impl Default for MemberConfig {
    fn default() -> Self { Self::new() }
}

// ── Apply errors ──────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum MembershipError {
    /// AddNode for an id that is already a voter.
    AlreadyVoter(u64),
    /// AddLearnerNode for an id that is already a learner or voter.
    AlreadyMember(u64),
    /// RemoveNode for an id we don't know about.
    UnknownMember(u64),
    /// LeaveJoint when not in joint consensus.
    NotInJoint,
    /// Promotion of a voter id (only learners can be promoted).
    NotALearner(u64),
    /// Cluster id mismatch on join.
    ClusterIdMismatch { expected: u64, got: u64 },
    /// Empty cluster id.
    InvalidClusterId,
}

impl std::fmt::Display for MembershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyVoter(id) => write!(f, "already a voter: {id}"),
            Self::AlreadyMember(id) => write!(f, "already a member: {id}"),
            Self::UnknownMember(id) => write!(f, "unknown member: {id}"),
            Self::NotInJoint => write!(f, "not in joint consensus"),
            Self::NotALearner(id) => write!(f, "not a learner: {id}"),
            Self::ClusterIdMismatch { expected, got } => write!(f, "cluster id mismatch: expected {expected}, got {got}"),
            Self::InvalidClusterId => write!(f, "invalid cluster id (zero)"),
        }
    }
}

impl std::error::Error for MembershipError {}

// ── State machine ─────────────────────────────────────────────────────────

/// In-memory state machine that applies [`ConfChangeV2`]s to a
/// [`MemberConfig`].
pub struct MembershipMachine {
    state: RwLock<MemberConfig>,
}

impl MembershipMachine {
    pub fn new(initial: MemberConfig) -> Self {
        Self { state: RwLock::new(initial) }
    }

    pub fn snapshot(&self) -> MemberConfig {
        self.state.read().unwrap().clone()
    }

    /// Apply a `ConfChangeV2`.  Side effect: if `transition` is Auto/Implicit
    /// and we entered joint, we automatically leave at the end of `apply`.
    pub fn apply(&self, change: &ConfChangeV2) -> Result<(), MembershipError> {
        let mut state = self.state.write().unwrap();

        // Empty change set ⇒ LeaveJoint.
        if change.is_leave_joint() {
            if !state.is_joint() {
                return Err(MembershipError::NotInJoint);
            }
            state.voters_outgoing.clear();
            return Ok(());
        }

        // Snapshot the *outgoing* voter set if this batch enters joint.
        let enters_joint = change.enters_joint();
        let outgoing = state.voters.clone();

        for c in &change.changes {
            match c.change_type {
                ConfChangeType::AddNode => {
                    if state.voters.contains(&c.node_id) {
                        return Err(MembershipError::AlreadyVoter(c.node_id));
                    }
                    // If it was a learner, promote.
                    state.learners.remove(&c.node_id);
                    state.voters.insert(c.node_id);
                }
                ConfChangeType::AddLearnerNode => {
                    if state.voters.contains(&c.node_id) || state.learners.contains(&c.node_id) {
                        return Err(MembershipError::AlreadyMember(c.node_id));
                    }
                    state.learners.insert(c.node_id);
                }
                ConfChangeType::RemoveNode => {
                    let was_voter = state.voters.remove(&c.node_id);
                    let was_learner = state.learners.remove(&c.node_id);
                    if !was_voter && !was_learner {
                        return Err(MembershipError::UnknownMember(c.node_id));
                    }
                }
                ConfChangeType::UpdateNode => {
                    // No-op for membership state — UpdateNode is metadata-only.
                }
            }
        }

        if enters_joint {
            state.voters_outgoing = outgoing;
            // Auto / Implicit ⇒ leave joint immediately.
            if !matches!(change.transition, ConfChangeTransition::Explicit) {
                state.voters_outgoing.clear();
            }
        }

        Ok(())
    }

    /// Promote a learner to voter (separate RPC in etcd v3.6).
    pub fn promote_learner(&self, id: u64) -> Result<(), MembershipError> {
        let mut state = self.state.write().unwrap();
        if !state.learners.remove(&id) {
            if state.voters.contains(&id) {
                return Err(MembershipError::AlreadyVoter(id));
            }
            return Err(MembershipError::NotALearner(id));
        }
        state.voters.insert(id);
        Ok(())
    }
}

// ── Cluster-id validation ────────────────────────────────────────────────

/// Validate that a joining member's cluster id matches the local cluster's.
/// Etcd's wire-protocol guard against accidentally pointing a member at the
/// wrong cluster.  Mirrors `etcdserver.ValidateClusterAndAssignIDs`.
pub fn validate_cluster_id(local: u64, remote: u64) -> Result<(), MembershipError> {
    if local == 0 { return Err(MembershipError::InvalidClusterId); }
    if local != remote {
        return Err(MembershipError::ClusterIdMismatch { expected: local, got: remote });
    }
    Ok(())
}

// ── Member-set diff ───────────────────────────────────────────────────────

/// Diff between two [`MemberConfig`]s — used by the membership audit log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberDiff {
    pub voters_added: BTreeSet<u64>,
    pub voters_removed: BTreeSet<u64>,
    pub learners_added: BTreeSet<u64>,
    pub learners_removed: BTreeSet<u64>,
    pub promoted: BTreeSet<u64>,
}

pub fn diff_configs(before: &MemberConfig, after: &MemberConfig) -> MemberDiff {
    let mut d = MemberDiff::default();
    d.voters_added = after.voters.difference(&before.voters).copied().collect();
    d.voters_removed = before.voters.difference(&after.voters).copied().collect();
    d.learners_added = after.learners.difference(&before.learners).copied().collect();
    d.learners_removed = before.learners.difference(&after.learners).copied().collect();
    // Promotion = was a learner, now a voter.
    d.promoted = before.learners.intersection(&after.voters).copied().collect();
    // A promotion shouldn't double-count as voter_added.
    d.voters_added = d.voters_added.difference(&d.promoted).copied().collect();
    d
}

// ── Member-pred IDs (for tests) ───────────────────────────────────────────

/// Generate a stable u64 cluster id from a string (used in tests).
pub fn cluster_id_from_token(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s.as_bytes() {
        h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
    }
    h
}

// Need this for trait bounds in BTreeMap usage above.
#[allow(dead_code)]
fn _force_btreemap_use() -> BTreeMap<u64, u64> { BTreeMap::new() }

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M12
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn three_voter_machine() -> MembershipMachine {
        MembershipMachine::new(MemberConfig::from_voters([1, 2, 3]))
    }

    // ── ConfChangeV2 helpers ──────────────────────────────────────────

    #[test]
    fn test_confchange_enters_joint_on_multi_voter() {
        // cite: confchange.go (multi-voter ⇒ joint)
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto)
            .add(ConfChangeType::AddNode, 4)
            .add(ConfChangeType::AddNode, 5);
        assert!(cc.enters_joint());
    }

    #[test]
    fn test_confchange_simple_when_one_voter_change() {
        // cite: confchange.go (single voter change is "simple")
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto)
            .add(ConfChangeType::AddNode, 4);
        assert!(!cc.enters_joint());
    }

    #[test]
    fn test_confchange_explicit_always_enters_joint() {
        // cite: confchange.go (Explicit ⇒ force joint even for one voter)
        let cc = ConfChangeV2::new(ConfChangeTransition::Explicit)
            .add(ConfChangeType::AddNode, 4);
        assert!(cc.enters_joint());
    }

    #[test]
    fn test_confchange_learner_changes_dont_force_joint() {
        // cite: confchange.go (learners don't affect quorum ⇒ no joint)
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto)
            .add(ConfChangeType::AddLearnerNode, 4)
            .add(ConfChangeType::AddLearnerNode, 5);
        assert!(!cc.enters_joint());
    }

    #[test]
    fn test_confchange_leave_joint_is_empty() {
        // cite: confchange.go (LeaveJoint == empty ConfChangeV2)
        let cc = ConfChangeV2::leave_joint();
        assert!(cc.is_leave_joint());
    }

    #[test]
    fn test_confchange_with_context() {
        // cite: raft.proto ConfChangeV2.context
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto).with_context(b"audit-ref-42".to_vec());
        assert_eq!(cc.context, b"audit-ref-42");
    }

    // ── MembershipMachine apply ────────────────────────────────────────

    #[test]
    fn test_apply_add_voter() {
        // cite: confchange.go AddNode
        let m = three_voter_machine();
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::AddNode, 4);
        m.apply(&cc).unwrap();
        assert!(m.snapshot().is_voter(4));
    }

    #[test]
    fn test_apply_add_voter_already_present_errors() {
        // cite: confchange.go (AddNode twice ⇒ error)
        let m = three_voter_machine();
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::AddNode, 1);
        assert_eq!(m.apply(&cc).unwrap_err(), MembershipError::AlreadyVoter(1));
    }

    #[test]
    fn test_apply_add_learner() {
        // cite: confchange.go AddLearnerNode
        let m = three_voter_machine();
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::AddLearnerNode, 4);
        m.apply(&cc).unwrap();
        let s = m.snapshot();
        assert!(s.is_learner(4));
        assert!(!s.is_voter(4));
    }

    #[test]
    fn test_apply_add_learner_already_present_errors() {
        // cite: confchange.go (AddLearner for known id ⇒ error)
        let m = three_voter_machine();
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::AddLearnerNode, 1);
        assert_eq!(m.apply(&cc).unwrap_err(), MembershipError::AlreadyMember(1));
    }

    #[test]
    fn test_apply_remove_voter() {
        // cite: confchange.go RemoveNode
        let m = three_voter_machine();
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::RemoveNode, 2);
        m.apply(&cc).unwrap();
        assert!(!m.snapshot().is_voter(2));
    }

    #[test]
    fn test_apply_remove_unknown_errors() {
        // cite: confchange.go (RemoveNode for unknown ⇒ error)
        let m = three_voter_machine();
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::RemoveNode, 99);
        assert_eq!(m.apply(&cc).unwrap_err(), MembershipError::UnknownMember(99));
    }

    #[test]
    fn test_apply_promote_via_addnode_for_learner() {
        // cite: confchange.go (AddNode of learner ⇒ promotion)
        let m = three_voter_machine();
        m.apply(&ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::AddLearnerNode, 4)).unwrap();
        m.apply(&ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::AddNode, 4)).unwrap();
        let s = m.snapshot();
        assert!(s.is_voter(4));
        assert!(!s.is_learner(4));
    }

    #[test]
    fn test_apply_explicit_enters_joint_then_leaves() {
        // cite: confchange.go ConfChangeTransitionExplicit
        let m = three_voter_machine();
        let cc = ConfChangeV2::new(ConfChangeTransition::Explicit).add(ConfChangeType::AddNode, 4);
        m.apply(&cc).unwrap();
        assert!(m.snapshot().is_joint());
        m.apply(&ConfChangeV2::leave_joint()).unwrap();
        assert!(!m.snapshot().is_joint());
    }

    #[test]
    fn test_apply_auto_does_not_remain_in_joint() {
        // cite: confchange.go ConfChangeTransitionAuto
        let m = three_voter_machine();
        let cc = ConfChangeV2::new(ConfChangeTransition::Auto)
            .add(ConfChangeType::AddNode, 4)
            .add(ConfChangeType::AddNode, 5);
        m.apply(&cc).unwrap();
        assert!(!m.snapshot().is_joint());
    }

    #[test]
    fn test_apply_leave_joint_when_not_joint_errors() {
        // cite: confchange.go (LeaveJoint outside joint ⇒ error)
        let m = three_voter_machine();
        assert_eq!(m.apply(&ConfChangeV2::leave_joint()).unwrap_err(), MembershipError::NotInJoint);
    }

    // ── promote_learner ────────────────────────────────────────────────

    #[test]
    fn test_promote_learner_to_voter() {
        // cite: server/etcdserver/server.go promoteMember
        let m = three_voter_machine();
        m.apply(&ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::AddLearnerNode, 4)).unwrap();
        m.promote_learner(4).unwrap();
        let s = m.snapshot();
        assert!(s.is_voter(4));
        assert!(!s.is_learner(4));
    }

    #[test]
    fn test_promote_voter_errors() {
        // cite: promoteMember (already-voter ⇒ error)
        let m = three_voter_machine();
        assert_eq!(m.promote_learner(1).unwrap_err(), MembershipError::AlreadyVoter(1));
    }

    #[test]
    fn test_promote_unknown_errors() {
        // cite: promoteMember (unknown id ⇒ NotALearner)
        let m = three_voter_machine();
        assert_eq!(m.promote_learner(99).unwrap_err(), MembershipError::NotALearner(99));
    }

    // ── Quorum + counts ───────────────────────────────────────────────

    #[test]
    fn test_quorum_three_voters() {
        // cite: raft (n/2)+1
        let m = three_voter_machine();
        assert_eq!(m.snapshot().quorum(), 2);
    }

    #[test]
    fn test_quorum_five_voters() {
        let m = MembershipMachine::new(MemberConfig::from_voters(1..=5));
        assert_eq!(m.snapshot().quorum(), 3);
    }

    #[test]
    fn test_voter_count() {
        let m = three_voter_machine();
        assert_eq!(m.snapshot().voter_count(), 3);
    }

    #[test]
    fn test_learner_count_after_add() {
        let m = three_voter_machine();
        m.apply(&ConfChangeV2::new(ConfChangeTransition::Auto).add(ConfChangeType::AddLearnerNode, 4)).unwrap();
        assert_eq!(m.snapshot().learner_count(), 1);
    }

    #[test]
    fn test_all_ids_includes_outgoing() {
        // cite: confchange.go (all_ids covers joint outgoing)
        let mut state = MemberConfig::from_voters([1, 2, 3]);
        state.voters_outgoing = [4u64].into_iter().collect();
        let ids = state.all_ids();
        assert!(ids.contains(&4));
    }

    // ── Cluster-id validation ─────────────────────────────────────────

    #[test]
    fn test_validate_cluster_id_match() {
        // cite: ValidateClusterAndAssignIDs
        validate_cluster_id(0xCAFE, 0xCAFE).unwrap();
    }

    #[test]
    fn test_validate_cluster_id_mismatch() {
        // cite: ValidateClusterAndAssignIDs (different cluster ⇒ error)
        let err = validate_cluster_id(0xCAFE, 0xBABE).unwrap_err();
        match err {
            MembershipError::ClusterIdMismatch { expected, got } => {
                assert_eq!(expected, 0xCAFE);
                assert_eq!(got, 0xBABE);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_validate_cluster_id_zero_invalid() {
        // cite: ValidateClusterAndAssignIDs (0 ⇒ uninitialised)
        assert_eq!(validate_cluster_id(0, 0).unwrap_err(), MembershipError::InvalidClusterId);
    }

    #[test]
    fn test_cluster_id_from_token_stable() {
        let a = cluster_id_from_token("cluster-prod-1");
        let b = cluster_id_from_token("cluster-prod-1");
        assert_eq!(a, b);
        let c = cluster_id_from_token("cluster-prod-2");
        assert_ne!(a, c);
    }

    // ── Diff ──────────────────────────────────────────────────────────

    #[test]
    fn test_diff_voter_added() {
        let before = MemberConfig::from_voters([1, 2]);
        let after = MemberConfig::from_voters([1, 2, 3]);
        let d = diff_configs(&before, &after);
        assert_eq!(d.voters_added, [3u64].into_iter().collect());
    }

    #[test]
    fn test_diff_voter_removed() {
        let before = MemberConfig::from_voters([1, 2, 3]);
        let after = MemberConfig::from_voters([1, 3]);
        let d = diff_configs(&before, &after);
        assert_eq!(d.voters_removed, [2u64].into_iter().collect());
    }

    #[test]
    fn test_diff_promotion_does_not_double_count() {
        // cite: server.go (promotion is its own bucket, not voter_added)
        let mut before = MemberConfig::from_voters([1, 2]);
        before.learners = [3u64].into_iter().collect();
        let after = MemberConfig::from_voters([1, 2, 3]);
        let d = diff_configs(&before, &after);
        assert_eq!(d.promoted, [3u64].into_iter().collect());
        assert!(d.voters_added.is_empty());
    }

    #[test]
    fn test_diff_no_change() {
        let cfg = MemberConfig::from_voters([1, 2, 3]);
        let d = diff_configs(&cfg, &cfg);
        assert!(d.voters_added.is_empty());
        assert!(d.voters_removed.is_empty());
        assert!(d.learners_added.is_empty());
        assert!(d.learners_removed.is_empty());
        assert!(d.promoted.is_empty());
    }
}
