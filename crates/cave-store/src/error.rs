use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("key not found")]
    KeyNotFound,
    #[error("key already exists")]
    KeyExists,
    #[error("revision {0} has been compacted")]
    RevisionCompacted(i64),
    #[error("lease {0} not found")]
    LeaseNotFound(i64),
    #[error("lease {0} has expired")]
    LeaseExpired(i64),
    #[error("bucket not found: {0}")]
    BucketNotFound(String),
    #[error("bucket already exists: {0}")]
    BucketExists(String),
    #[error("object not found: {0}/{1}")]
    ObjectNotFound(String, String),
    #[error("upload not found: {0}")]
    UploadNotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("authentication failed")]
    AuthFailed,
    #[error("permission denied")]
    PermissionDenied,
    #[error("WAL corrupted: {0}")]
    WalCorrupted(String),
    #[error("transaction failed")]
    TxnFailed,
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("user not found: {0}")]
    UserNotFound(String),
    #[error("user already exists: {0}")]
    UserExists(String),
    #[error("role not found: {0}")]
    RoleNotFound(String),
    #[error("role already exists: {0}")]
    RoleExists(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;
