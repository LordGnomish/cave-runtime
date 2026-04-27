//! Error types shared by the S3 and Azure surfaces.

use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BlobError {
    #[error("bucket not found: {0}")]
    BucketNotFound(String),
    #[error("bucket already exists: {0}")]
    BucketExists(String),
    #[error("object not found: {0}/{1}")]
    ObjectNotFound(String, String),
    #[error("invalid object key: {0}")]
    InvalidKey(String),
    #[error("invalid bucket name: {0}")]
    InvalidBucketName(String),
    #[error("multipart upload not found: {0}")]
    UploadNotFound(String),
    #[error("invalid part number: {0}")]
    InvalidPartNumber(i32),
    #[error("part missing in complete: {0}")]
    MissingPart(i32),
    #[error("etag mismatch on part {0}: got {1}, expected {2}")]
    EtagMismatch(i32, String, String),
    #[error("expired: {0}")]
    Expired(DateTime<Utc>),
    #[error("signature mismatch")]
    SignatureMismatch,
    #[error("tenant denied: {0}")]
    TenantDenied(String),
    #[error("invalid lifecycle rule: {0}")]
    InvalidLifecycleRule(String),
    #[error("immutable lock prevents delete")]
    Immutable,
}

pub type BlobResult<T> = Result<T, BlobError>;
