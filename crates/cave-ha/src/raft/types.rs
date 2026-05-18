// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Raft term number (monotonically increasing).
pub type Term = u64;
/// Log index (1-based; 0 = invalid).
pub type LogIndex = u64;
/// Node identifier.
pub type NodeId = u64;

/// Raft node role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// Not yet participating (startup / learner).
    Follower,
    /// Soliciting pre-votes (pre-vote phase).
    PreCandidate,
    /// Campaigning for leadership.
    Candidate,
    /// Cluster leader.
    Leader,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Follower => write!(f, "Follower"),
            Role::PreCandidate => write!(f, "PreCandidate"),
            Role::Candidate => write!(f, "Candidate"),
            Role::Leader => write!(f, "Leader"),
        }
    }
}

/// Hard state that must be persisted before responding to any RPC.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HardState {
    pub term: Term,
    pub voted_for: Option<NodeId>,
    pub commit: LogIndex,
}

/// Snapshot metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotMeta {
    pub index: LogIndex,
    pub term: Term,
    pub membership: MembershipConfig,
}

/// Membership configuration supporting joint consensus.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MembershipConfig {
    /// Current voter set (C_new during joint consensus).
    pub voters: BTreeSet<NodeId>,
    /// Non-voting members that replicate but don't vote.
    pub learners: BTreeSet<NodeId>,
    /// Old voter set (Some during joint consensus transition).
    pub voters_outgoing: Option<BTreeSet<NodeId>>,
    /// True when this is a joint consensus entry (C_old,new).
    pub auto_leave: bool,
}

impl MembershipConfig {
    pub fn single(id: NodeId) -> Self {
        let mut voters = BTreeSet::new();
        voters.insert(id);
        Self { voters, ..Default::default() }
    }

    /// True if in joint consensus phase.
    pub fn is_joint(&self) -> bool {
        self.voters_outgoing.is_some()
    }

    /// Quorum size for a voter set.
    pub fn quorum(set_size: usize) -> usize {
        set_size / 2 + 1
    }

    /// Check if vote count achieves quorum from required sets.
    pub fn has_quorum(&self, votes: &BTreeSet<NodeId>) -> bool {
        let new_quorum = Self::quorum(self.voters.len());
        let new_votes = self.voters.iter().filter(|id| votes.contains(id)).count();
        if new_votes < new_quorum {
            return false;
        }
        // During joint consensus, must also achieve quorum in old set.
        if let Some(ref old) = self.voters_outgoing {
            let old_quorum = Self::quorum(old.len());
            let old_votes = old.iter().filter(|id| votes.contains(id)).count();
            if old_votes < old_quorum {
                return false;
            }
        }
        true
    }

    /// All nodes (voters + learners, both old and new).
    pub fn all_nodes(&self) -> BTreeSet<NodeId> {
        let mut all = self.voters.clone();
        all.extend(self.learners.iter().copied());
        if let Some(ref old) = self.voters_outgoing {
            all.extend(old.iter().copied());
        }
        all
    }

    /// All voters (both old and new sets during joint).
    pub fn all_voters(&self) -> BTreeSet<NodeId> {
        let mut v = self.voters.clone();
        if let Some(ref old) = self.voters_outgoing {
            v.extend(old.iter().copied());
        }
        v
    }
}

/// Cluster node information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: NodeId,
    pub addr: String,
    pub is_learner: bool,
}

/// Snapshot of node status for external visibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatus {
    pub id: NodeId,
    pub role: String,
    pub term: Term,
    pub commit_index: LogIndex,
    pub last_applied: LogIndex,
    pub leader_id: Option<NodeId>,
    pub membership: MembershipConfig,
    pub last_log_index: LogIndex,
    pub last_log_term: Term,
}

/// Entry type discriminator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EntryType {
    /// Normal client data.
    Normal,
    /// Membership configuration change.
    MembershipChange,
    /// No-op barrier entry (written by new leader to commit previous term entries).
    Barrier,
}
