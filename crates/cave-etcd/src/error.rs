//! Error types for cave-etcd.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EtcdError {
    #[error("key not found: {0}")]
    KeyNotFound(String),

    #[error("revision compacted: requested {requested}, compacted at {compacted}")]
    RevisionCompacted { requested: u64, compacted: u64 },

    #[error("lease not found: {0}")]
    LeaseNotFound(i64),

    #[error("lease expired: {0}")]
    LeaseExpired(i64),

    #[error("compare failed: key {0}")]
    CompareFailed(String),

    #[error("watch cancelled: {0}")]
    WatchCancelled(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type EtcdResult<T> = Result<T, EtcdError>;
