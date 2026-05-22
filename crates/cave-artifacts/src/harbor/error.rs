// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/server/registry/error/error.go
//! Registry error types with Docker V2 error body serialisation.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("name unknown or not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("access denied: {0}")]
    Forbidden(String),
    #[error("digest mismatch: expected {expected}, got {got}")]
    DigestMismatch { expected: String, got: String },
    #[error("manifest invalid: {0}")]
    InvalidManifest(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("upload session not found: {0}")]
    UploadNotFound(String),
    #[error("policy violation: {0}")]
    PolicyViolation(String),
    #[error("replication error: {0}")]
    Replication(String),
    #[error("repository name invalid")]
    NameInvalid,
    #[error("blob unknown")]
    BlobUnknown,
    #[error("manifest unknown")]
    ManifestUnknown,
    #[error("tag invalid")]
    TagInvalid,
    #[error("method not allowed")]
    MethodNotAllowed,
    #[error("unsupported path")]
    UnsupportedPath,
}

impl IntoResponse for RegistryError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            RegistryError::NotFound | RegistryError::ManifestUnknown => {
                (StatusCode::NOT_FOUND, "NAME_UNKNOWN")
            }
            RegistryError::BlobUnknown => (StatusCode::NOT_FOUND, "BLOB_UNKNOWN"),
            RegistryError::Unauthorized => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            RegistryError::Forbidden(_) => (StatusCode::FORBIDDEN, "DENIED"),
            RegistryError::DigestMismatch { .. } => (StatusCode::BAD_REQUEST, "DIGEST_INVALID"),
            RegistryError::InvalidManifest(_) => (StatusCode::BAD_REQUEST, "MANIFEST_INVALID"),
            RegistryError::Storage(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
            RegistryError::UploadNotFound(_) => (StatusCode::NOT_FOUND, "BLOB_UPLOAD_UNKNOWN"),
            RegistryError::PolicyViolation(_) => (StatusCode::METHOD_NOT_ALLOWED, "DENIED"),
            RegistryError::Replication(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
            RegistryError::NameInvalid => (StatusCode::BAD_REQUEST, "NAME_INVALID"),
            RegistryError::TagInvalid => (StatusCode::BAD_REQUEST, "TAG_INVALID"),
            RegistryError::MethodNotAllowed => (StatusCode::METHOD_NOT_ALLOWED, "UNSUPPORTED"),
            RegistryError::UnsupportedPath => (StatusCode::NOT_FOUND, "NAME_UNKNOWN"),
        };
        let body = serde_json::json!({
            "errors": [{ "code": code, "message": self.to_string(), "detail": null }]
        });
        (status, Json(body)).into_response()
    }
}
