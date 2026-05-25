// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Top-level error type for cave-iceberg.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("already exists: {0}")]
    AlreadyExists(String),

    #[error("invalid schema: {0}")]
    InvalidSchema(String),

    #[error("invalid metadata: {0}")]
    InvalidMetadata(String),

    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("serialization error: {0}")]
    Serde(String),

    #[error("unsupported format-version: {0}")]
    UnsupportedFormatVersion(i32),

    #[error("snapshot not found: {0}")]
    SnapshotNotFound(i64),
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Serde(value.to_string())
    }
}
