// SPDX-License-Identifier: AGPL-3.0-or-later
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

    #[error("too many ops in transaction: {ops} > max {max}")]
    TooManyTxnOps { ops: usize, max: usize },

    // ── v3.6: raft membership / joint consensus ──────────────────────────────
    #[error("member is not a learner: {0}")]
    MemberNotLearner(u64),

    #[error("learner not ready for promotion: {0}")]
    LearnerNotReady(u64),

    #[error("joint consensus already in progress")]
    JointConfigInProgress,

    #[error("no joint consensus in progress")]
    NoJointConfig,

    #[error("would break voting quorum")]
    WouldBreakQuorum,

    // ── v3.6: lease ──────────────────────────────────────────────────────────
    #[error("invalid lease ttl: {0}")]
    InvalidLeaseTtl(i64),

    #[error("lease already exists: {0}")]
    LeaseAlreadyExists(i64),

    // ── v3.6: compaction ─────────────────────────────────────────────────────
    #[error("compaction revision {requested} exceeds current revision {current}")]
    CompactionFutureRevision { requested: u64, current: u64 },

    // ── v3.6: snapshot ───────────────────────────────────────────────────────
    #[error("snapshot checksum mismatch: expected {expected}, got {actual}")]
    SnapshotChecksumMismatch { expected: String, actual: String },

    #[error("snapshot decode error: {0}")]
    SnapshotDecode(String),

    // ── v3.6: watch ─────────────────────────────────────────────────────────
    #[error("watch not found: {0}")]
    WatchNotFound(i64),

    // ── v3.6 deeper-002: raft / read consistency ───────────────────────────
    #[error("not leader: current_term={term}, leader={leader:?}")]
    NotLeader { term: u64, leader: Option<u64> },

    #[error("leader lease expired: granted_at={granted_at}, ttl_ms={ttl_ms}")]
    LeaderLeaseExpired { granted_at: i64, ttl_ms: u64 },

    #[error("read index timeout: index={index}, applied={applied}")]
    ReadIndexTimeout { index: u64, applied: u64 },

    #[error("quorum lost: required={required}, healthy={healthy}")]
    QuorumLost { required: usize, healthy: usize },

    #[error("pre-vote rejected: reason={0}")]
    PreVoteRejected(String),
}

pub type EtcdResult<T> = Result<T, EtcdError>;
