// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared types for the `/admin/iceberg` page set.
//!
//! Mirrors Apache Iceberg's REST-catalog payload shape (table → snapshots
//! → manifests → partition spec) closely enough that a real backend can
//! drop in via [`crate::admin::runtime_client`] without changing the
//! Portal views.

use crate::admin::types::TenantId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcebergTable {
    pub tenant: TenantId,
    pub namespace: String,
    pub name: String,
    pub location: String,
    /// `format_version` 1 or 2.
    pub format_version: u32,
    pub current_snapshot_id: Option<i64>,
    pub schema_id: u32,
    pub last_updated_ms: i64,
    pub row_count: u64,
    pub file_count: u32,
    pub total_data_files_bytes: u64,
    /// Partition spec id this table is currently using.
    pub partition_spec_id: u32,
}

impl IcebergTable {
    pub fn fqn(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcebergSnapshot {
    pub tenant: TenantId,
    pub table_fqn: String,
    pub snapshot_id: i64,
    pub parent_snapshot_id: Option<i64>,
    pub sequence_number: u64,
    pub timestamp_ms: i64,
    /// "append" | "overwrite" | "delete" | "replace"
    pub operation: String,
    pub manifest_list: String,
    pub summary_added_records: u64,
    pub summary_added_files: u32,
    pub summary_deleted_records: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionField {
    pub field_id: u32,
    pub source_id: u32,
    pub name: String,
    /// "identity" | "year" | "month" | "day" | "hour" | "bucket[N]" | "truncate[N]"
    pub transform: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionSpec {
    pub tenant: TenantId,
    pub table_fqn: String,
    pub spec_id: u32,
    pub fields: Vec<PartitionField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaField {
    pub id: u32,
    pub name: String,
    /// "int" | "long" | "string" | "double" | "boolean" | "timestamptz" | "struct" ...
    pub data_type: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcebergSchema {
    pub tenant: TenantId,
    pub table_fqn: String,
    pub schema_id: u32,
    pub fields: Vec<SchemaField>,
    pub identifier_field_ids: Vec<u32>,
    pub previous_schema_id: Option<u32>,
    pub last_updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcebergManifest {
    pub tenant: TenantId,
    pub table_fqn: String,
    pub snapshot_id: i64,
    pub manifest_path: String,
    /// "data" | "delete"
    pub content: String,
    pub partition_spec_id: u32,
    pub added_files_count: u32,
    pub existing_files_count: u32,
    pub deleted_files_count: u32,
    pub added_rows_count: u64,
    pub existing_rows_count: u64,
    pub deleted_rows_count: u64,
    pub min_sequence_number: u64,
    pub manifest_length_bytes: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IcebergViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("table {0} not found")]
    TableNotFound(String),
    #[error("snapshot {0} not found")]
    SnapshotNotFound(i64),
}
