// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error types for cave-iceberg.
//!
//! Mirrors the failure modes of apache/iceberg-rust crates/iceberg/src/error.rs.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum IcebergError {
    #[error("schema error: {0}")]
    Schema(String),
    #[error("partition spec error: {0}")]
    PartitionSpec(String),
    #[error("manifest error: {0}")]
    Manifest(String),
    #[error("snapshot error: {0}")]
    Snapshot(String),
    #[error("table metadata error: {0}")]
    TableMetadata(String),
    #[error("invalid tenant_id: {0}")]
    InvalidTenant(String),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

pub type IcebergResult<T> = Result<T, IcebergError>;
