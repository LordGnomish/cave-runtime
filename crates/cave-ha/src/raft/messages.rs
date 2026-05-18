// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};
use crate::raft::log::LogEntry;
use crate::raft::types::{LogIndex, NodeId, SnapshotMeta, Term};

/// All messages exchanged between Raft nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RaftMessage {
    RequestVote(RequestVote),
    RequestVoteReply(RequestVoteReply),
    AppendEntries(AppendEntries),
    AppendEntriesReply(AppendEntriesReply),
    InstallSnapshot(InstallSnapshot),
    InstallSnapshotReply(InstallSnapshotReply),
    /// Sent by leader to transfer leadership to a specific node.
    TimeoutNow(TimeoutNow),
    /// ReadIndex request forwarded to leader.
    ReadIndexRequest(ReadIndexRequest),
    /// Leader replies with safe read index.
    ReadIndexReply(ReadIndexReply),
}

impl RaftMessage {
    pub fn term(&self) -> Term {
        match self {
            RaftMessage::RequestVote(m) => m.term,
            RaftMessage::RequestVoteReply(m) => m.term,
            RaftMessage::AppendEntries(m) => m.term,
            RaftMessage::AppendEntriesReply(m) => m.term,
            RaftMessage::InstallSnapshot(m) => m.term,
            RaftMessage::InstallSnapshotReply(m) => m.term,
            RaftMessage::TimeoutNow(m) => m.term,
            RaftMessage::ReadIndexRequest(m) => m.term,
            RaftMessage::ReadIndexReply(m) => m.term,
        }
    }
}

/// RequestVote RPC — doubles as PreVote when `pre_vote = true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestVote {
    pub term: Term,
    pub candidate_id: NodeId,
    pub last_log_index: LogIndex,
    pub last_log_term: Term,
    /// When true this is a pre-vote (term is candidate's current_term + 1,
    /// not yet persisted).
    pub pre_vote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestVoteReply {
    pub term: Term,
    pub vote_granted: bool,
    /// Echo the pre_vote flag so the sender can correlate.
    pub pre_vote: bool,
}

/// AppendEntries RPC (also used as heartbeat when entries is empty).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEntries {
    pub term: Term,
    pub leader_id: NodeId,
    pub prev_log_index: LogIndex,
    pub prev_log_term: Term,
    pub entries: Vec<LogEntry>,
    pub leader_commit: LogIndex,
    /// Sequence number for pipelining / flow control.
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEntriesReply {
    pub term: Term,
    pub success: bool,
    /// On failure: the follower's conflict index for fast log backtracking.
    pub conflict_index: LogIndex,
    pub conflict_term: Option<Term>,
    /// Index of last entry replicated (when success).
    pub last_log_index: LogIndex,
    /// Echo for pipelining.
    pub seq: u64,
}

/// Snapshot installation — sent in chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSnapshot {
    pub term: Term,
    pub leader_id: NodeId,
    pub meta: SnapshotMeta,
    pub offset: u64,
    pub data: Vec<u8>,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSnapshotReply {
    pub term: Term,
    pub success: bool,
    pub bytes_stored: u64,
}

/// Leadership transfer: leader asks this node to immediately start election.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutNow {
    pub term: Term,
    pub from: NodeId,
}

/// Client or follower asks leader for the current commit index (ReadIndex protocol).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadIndexRequest {
    pub term: Term,
    pub from: NodeId,
    /// Opaque request ID echoed back.
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadIndexReply {
    pub term: Term,
    pub id: u64,
    pub read_index: LogIndex,
    pub success: bool,
}

/// Envelope wrapping a message with sender ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub from: NodeId,
    pub to: NodeId,
    pub msg: RaftMessage,
}
