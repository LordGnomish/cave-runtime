// SPDX-License-Identifier: AGPL-3.0-or-later
use thiserror::Error;
use tokio::sync::oneshot;

#[derive(Debug, Error)]
pub enum HaError {
    #[error("not leader: current leader is {leader_id:?}")]
    NotLeader { leader_id: Option<u64> },

    #[error("no quorum available")]
    NoQuorum,

    #[error("log compacted: requested index {requested} < snapshot index {snapshot}")]
    LogCompacted { requested: u64, snapshot: u64 },

    #[error("proposal dropped")]
    ProposalDropped,

    #[error("leadership transfer in progress")]
    TransferInProgress,

    #[error("node is learner, not voter")]
    IsLearner,

    #[error("membership change already pending")]
    MembershipChangePending,

    #[error("node {0} not found in cluster")]
    NodeNotFound(u64),

    #[error("raft error: {0}")]
    Raft(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("snapshot error: {0}")]
    Snapshot(String),

    #[error("dr error: {0}")]
    Dr(String),

    #[error("operation timed out")]
    Timeout,

    #[error("node is shutting down")]
    Shutdown,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type HaResult<T> = Result<T, HaError>;

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for HaError {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        HaError::Shutdown
    }
}

impl From<oneshot::error::RecvError> for HaError {
    fn from(_: oneshot::error::RecvError) -> Self {
        HaError::Shutdown
    }
}
