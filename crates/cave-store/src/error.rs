// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-store.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    // etcd errors
    #[error("key not found: {0}")]
    KeyNotFound(String),
    #[error("revision compacted: requested {requested}, compacted {compacted}")]
    RevisionCompacted { requested: i64, compacted: i64 },
    #[error("transaction compare failed")]
    TransactionFailed,
    #[error("lease not found: {0}")]
    LeaseNotFound(i64),
    #[error("lease expired: {0}")]
    LeaseExpired(i64),
    #[error("lease exists: {0}")]
    LeaseExists(i64),
    #[error("watch not found: {0}")]
    WatchNotFound(i64),
    #[error("auth not enabled")]
    AuthNotEnabled,
    #[error("auth already enabled")]
    AuthAlreadyEnabled,
    #[error("user not found: {0}")]
    UserNotFound(String),
    #[error("user already exists: {0}")]
    UserAlreadyExists(String),
    #[error("role not found: {0}")]
    RoleNotFound(String),
    #[error("role already exists: {0}")]
    RoleAlreadyExists(String),
    #[error("permission denied")]
    PermissionDenied,
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("cluster member not found: {0}")]
    MemberNotFound(u64),

    // S3 errors
    #[error("bucket not found: {0}")]
    BucketNotFound(String),
    #[error("bucket already exists: {0}")]
    BucketAlreadyExists(String),
    #[error("bucket not empty: {0}")]
    BucketNotEmpty(String),
    #[error("object not found: {bucket}/{key}")]
    ObjectNotFound { bucket: String, key: String },
    #[error("no such upload: {0}")]
    NoSuchUpload(String),
    #[error("invalid part: {0}")]
    InvalidPart(String),
    #[error("entity too small")]
    EntityTooSmall,
    #[error("invalid bucket name: {0}")]
    InvalidBucketName(String),
    #[error("invalid object key: {0}")]
    InvalidObjectKey(String),
    #[error("precondition failed")]
    PreconditionFailed,
    #[error("request expired")]
    RequestExpired,
    #[error("signature mismatch")]
    SignatureMismatch,
    #[error("encryption error: {0}")]
    EncryptionError(String),
    #[error("versioning not enabled for bucket: {0}")]
    VersioningNotEnabled(String),
    #[error("invalid lifecycle rule: {0}")]
    InvalidLifecycleRule(String),

    // Storage/IO errors
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

impl StoreError {
    pub fn s3_code(&self) -> &'static str {
        match self {
            StoreError::BucketNotFound(_) => "NoSuchBucket",
            StoreError::BucketAlreadyExists(_) => "BucketAlreadyExists",
            StoreError::BucketNotEmpty(_) => "BucketNotEmpty",
            StoreError::ObjectNotFound { .. } => "NoSuchKey",
            StoreError::NoSuchUpload(_) => "NoSuchUpload",
            StoreError::InvalidPart(_) => "InvalidPart",
            StoreError::EntityTooSmall => "EntityTooSmall",
            StoreError::InvalidBucketName(_) => "InvalidBucketName",
            StoreError::InvalidObjectKey(_) => "InvalidObjectKey",
            StoreError::PreconditionFailed => "PreconditionFailed",
            StoreError::RequestExpired => "RequestExpired",
            StoreError::SignatureMismatch => "SignatureDoesNotMatch",
            StoreError::EncryptionError(_) => "InvalidEncryptionAlgorithmError",
            StoreError::VersioningNotEnabled(_) => "InvalidBucketState",
            StoreError::PermissionDenied => "AccessDenied",
            _ => "InternalError",
        }
    }

    pub fn s3_status(&self) -> u16 {
        match self {
            StoreError::BucketNotFound(_) => 404,
            StoreError::ObjectNotFound { .. } => 404,
            StoreError::NoSuchUpload(_) => 404,
            StoreError::BucketAlreadyExists(_) => 409,
            StoreError::BucketNotEmpty(_) => 409,
            StoreError::PreconditionFailed => 412,
            StoreError::RequestExpired => 403,
            StoreError::SignatureMismatch => 403,
            StoreError::PermissionDenied => 403,
            StoreError::InvalidBucketName(_) => 400,
            StoreError::InvalidObjectKey(_) => 400,
            StoreError::InvalidPart(_) => 400,
            StoreError::EntityTooSmall => 400,
            _ => 500,
        }
    }
}
