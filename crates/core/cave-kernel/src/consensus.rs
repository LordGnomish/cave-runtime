// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Raft consensus primitives — log/state-machine/handle traits.
//!
//! Upstream cite: hashicorp/raft, etcd-io/raft. The CAVE kernel provides a
//! transport-agnostic *contract* for log replication and a leader-tracker so
//! that cave-ha (Raft node implementation) and downstream consumers
//! (cave-etcd, cave-apiserver) can wire against the same surface.
//!
//! This module deliberately ships *traits + small data types only* — concrete
//! Raft FSM/transport implementations live in cave-ha. The goal is to remove
//! ad-hoc copies of `RaftHandle`-shaped types from per-module client code.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

/// Monotonic, contiguous index assigned to each appended log entry. Index 0
/// is reserved for "no-op / before-any-entry" markers.
pub type LogIndex = u64;

/// Election term. Strictly increasing; a follower that observes a higher term
/// in any RPC must update and revert to follower state.
pub type Term = u64;

/// Stable identifier for a Raft peer. Implementations typically use a small
/// integer or UUID — both fit in a `String` for transport neutrality.
pub type NodeId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Follower,
    Candidate,
    Leader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub index: LogIndex,
    pub term: Term,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaderInfo {
    pub leader: Option<NodeId>,
    pub term: Term,
    pub role: Role,
}

#[derive(Debug, Error)]
pub enum ConsensusError {
    #[error("not the leader; current leader: {0:?}")]
    NotLeader(Option<NodeId>),
    #[error("log entry not found at index {0}")]
    LogNotFound(LogIndex),
    #[error("transport: {0}")]
    Transport(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("aborted: {0}")]
    Aborted(String),
}

pub type ConsensusResult<T> = Result<T, ConsensusError>;

/// Persistent log storage contract. Implementations must guarantee that
/// `append` is durable before returning success; readers must see entries in
/// strict index order.
#[async_trait]
pub trait LogStore: Send + Sync {
    async fn append(&self, entries: &[LogEntry]) -> ConsensusResult<()>;
    async fn get(&self, index: LogIndex) -> ConsensusResult<Option<LogEntry>>;
    async fn last_index(&self) -> ConsensusResult<LogIndex>;
    /// Truncate strictly after `index` (i.e. keep entries `[0..=index]`).
    async fn truncate_after(&self, index: LogIndex) -> ConsensusResult<()>;
}

/// Application of committed log entries to user state. The state machine is
/// the only authority on snapshot content.
#[async_trait]
pub trait StateMachine: Send + Sync {
    async fn apply(&self, entry: &LogEntry) -> ConsensusResult<Vec<u8>>;
    async fn snapshot(&self) -> ConsensusResult<Vec<u8>>;
    async fn restore(&self, snapshot: &[u8]) -> ConsensusResult<()>;
}

/// Client-facing handle to a Raft node — the one type cave-etcd /
/// cave-apiserver code should reach for. Implementations in cave-ha wrap a
/// running node and forward.
#[async_trait]
pub trait RaftHandle: Send + Sync {
    async fn propose(&self, data: Vec<u8>) -> ConsensusResult<LogIndex>;
    async fn read_index(&self) -> ConsensusResult<LogIndex>;
    async fn leader(&self) -> ConsensusResult<LeaderInfo>;
    async fn node_id(&self) -> NodeId;
}

/// Type-erased dynamic handle for crates that prefer `Arc<dyn RaftHandle>`
/// over generics.
pub type DynRaftHandle = Arc<dyn RaftHandle>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_round_trips_through_serde() {
        let json = serde_json::to_string(&Role::Leader).unwrap();
        assert_eq!(json, "\"Leader\"");
        let back: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Role::Leader);
    }

    #[test]
    fn leader_info_default_term_is_zero() {
        let li = LeaderInfo {
            leader: None,
            term: 0,
            role: Role::Follower,
        };
        assert_eq!(li.term, 0);
        assert!(li.leader.is_none());
    }

    #[test]
    fn consensus_error_not_leader_displays() {
        let e = ConsensusError::NotLeader(Some("node-2".into()));
        let s = format!("{e}");
        assert!(s.contains("not the leader"));
        assert!(s.contains("node-2"));
    }

    #[test]
    fn log_entry_serializes_with_byte_data() {
        let e = LogEntry {
            index: 42,
            term: 7,
            data: b"hello".to_vec(),
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["index"], 42);
        assert_eq!(json["term"], 7);
    }
}
