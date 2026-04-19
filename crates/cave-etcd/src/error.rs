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

    #[error("auth not enabled")]
    AuthNotEnabled,

    #[error("auth already enabled")]
    AuthAlreadyEnabled,

    #[error("user already exists: {0}")]
    UserAlreadyExists(String),

    #[error("user not found: {0}")]
    UserNotFound(String),

    #[error("role already exists: {0}")]
    RoleAlreadyExists(String),

    #[error("role not found: {0}")]
    RoleNotFound(String),

    #[error("invalid password")]
    InvalidPassword,

    #[error("permission denied")]
    PermissionDenied,

    #[error("member not found: {0}")]
    MemberNotFound(u64),

    #[error("invalid token")]
    InvalidToken,

    #[error("permission already granted")]
    PermissionAlreadyGranted,

    #[error("role not granted to user")]
    RoleNotGranted,
}

pub type EtcdResult<T> = Result<T, EtcdError>;
