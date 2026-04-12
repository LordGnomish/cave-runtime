#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("bucket not found: {0}")]
    BucketNotFound(String),
    #[error("bucket already exists: {0}")]
    BucketExists(String),
    #[error("object not found: {0}")]
    ObjectNotFound(String),
    #[error("invalid bucket name: {0}")]
    InvalidBucket(String),
    #[error("access denied")]
    AccessDenied,
    #[error("multipart upload not found: {0}")]
    UploadNotFound(String),
    #[error("invalid part")]
    InvalidPart,
    #[error("presign error: {0}")]
    PresignError(String),
    #[error("encryption error: {0}")]
    EncryptionError(String),
    #[error("lifecycle error: {0}")]
    LifecycleError(String),
}

pub type StoreResult<T> = Result<T, StoreError>;
