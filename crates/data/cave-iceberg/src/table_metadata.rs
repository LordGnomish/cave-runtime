// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg TableMetadata (v2 — MVP; v3 partial).
//!
//! Upstream: `crates/iceberg/src/spec/table_metadata.rs`
//! Spec: <https://iceberg.apache.org/spec/#table-metadata>

use crate::error::{Error, Result};
use crate::schema::Schema;
use crate::snapshot::{Snapshot, SnapshotRef};
use crate::sort_order::SortOrder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(into = "i32", try_from = "i32")]
pub enum FormatVersion {
    V1 = 1,
    V2 = 2,
    V3 = 3,
}

impl TryFrom<i32> for FormatVersion {
    type Error = Error;

    fn try_from(v: i32) -> Result<Self> {
        match v {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            3 => Ok(Self::V3),
            _ => Err(Error::UnsupportedFormatVersion(v)),
        }
    }
}

impl From<FormatVersion> for i32 {
    fn from(v: FormatVersion) -> Self {
        v as i32
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartitionSpec {
    pub spec_id: i32,
    pub fields: Vec<PartitionField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartitionField {
    pub source_id: i32,
    pub field_id: i32,
    pub name: String,
    pub transform: String,
}

impl Default for PartitionSpec {
    fn default() -> Self {
        Self {
            spec_id: 0,
            fields: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableMetadata {
    #[serde(rename = "format-version")]
    pub format_version: FormatVersion,
    #[serde(default)]
    pub table_uuid: String,
    pub location: String,
    #[serde(rename = "last-sequence-number", default)]
    pub last_sequence_number: i64,
    #[serde(rename = "last-updated-ms", default)]
    pub last_updated_ms: i64,
    #[serde(rename = "last-column-id", default)]
    pub last_column_id: i32,
    pub schemas: Vec<Schema>,
    #[serde(rename = "current-schema-id", default)]
    pub current_schema_id: i32,
    #[serde(rename = "partition-specs", default)]
    pub partition_specs: Vec<PartitionSpec>,
    #[serde(rename = "default-spec-id", default)]
    pub default_spec_id: i32,
    #[serde(rename = "last-partition-id", default)]
    pub last_partition_id: i32,
    #[serde(rename = "sort-orders", default)]
    pub sort_orders: Vec<SortOrder>,
    #[serde(rename = "default-sort-order-id", default)]
    pub default_sort_order_id: i32,
    #[serde(default)]
    pub properties: HashMap<String, String>,
    #[serde(
        rename = "current-snapshot-id",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub current_snapshot_id: Option<i64>,
    #[serde(default)]
    pub snapshots: Vec<Snapshot>,
    #[serde(rename = "snapshot-log", default)]
    pub snapshot_log: Vec<SnapshotLogEntry>,
    #[serde(rename = "metadata-log", default)]
    pub metadata_log: Vec<MetadataLogEntry>,
    #[serde(default)]
    pub refs: HashMap<String, SnapshotRef>,
    /// v3 row lineage — the `_row_id` to assign to the next added row.
    /// Advanced by each commit's `added-rows`. Always present in v3
    /// metadata; defaults to 0 for v1/v2 round-trips.
    #[serde(rename = "next-row-id", default)]
    pub next_row_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotLogEntry {
    #[serde(rename = "snapshot-id")]
    pub snapshot_id: i64,
    #[serde(rename = "timestamp-ms")]
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetadataLogEntry {
    #[serde(rename = "metadata-file")]
    pub metadata_file: String,
    #[serde(rename = "timestamp-ms")]
    pub timestamp_ms: i64,
}

impl TableMetadata {
    pub fn builder() -> TableMetadataBuilder {
        TableMetadataBuilder::default()
    }

    pub fn current_schema(&self) -> Option<&Schema> {
        self.schemas
            .iter()
            .find(|s| s.schema_id == self.current_schema_id)
    }

    pub fn current_snapshot(&self) -> Option<&Snapshot> {
        let id = self.current_snapshot_id?;
        self.snapshots.iter().find(|s| s.snapshot_id == id)
    }

    pub fn snapshot_by_id(&self, id: i64) -> Option<&Snapshot> {
        self.snapshots.iter().find(|s| s.snapshot_id == id)
    }

    /// Parse a metadata.json string.
    pub fn from_json(s: &str) -> Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Iceberg v3 row-lineage assignment for a committing snapshot.
    /// Stamps the snapshot's `first-row-id` from the table's current
    /// `next-row-id`, records `added-rows`, then advances `next-row-id`
    /// by that count. An empty commit (`added_rows == 0`) leaves
    /// `next-row-id` unchanged but still stamps `first-row-id`.
    pub fn assign_snapshot_row_ids(&mut self, snapshot: &mut Snapshot, added_rows: i64) {
        snapshot.first_row_id = Some(self.next_row_id);
        snapshot.added_rows = Some(added_rows);
        self.next_row_id += added_rows;
    }
}

#[derive(Debug, Default)]
pub struct TableMetadataBuilder {
    format_version: Option<FormatVersion>,
    table_uuid: Option<String>,
    location: Option<String>,
    schema: Option<Schema>,
}

impl TableMetadataBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn format_version(mut self, v: FormatVersion) -> Self {
        self.format_version = Some(v);
        self
    }

    pub fn location(mut self, l: impl Into<String>) -> Self {
        self.location = Some(l.into());
        self
    }

    pub fn schema(mut self, s: Schema) -> Self {
        self.schema = Some(s);
        self
    }

    pub fn build(self) -> Result<TableMetadata> {
        let schema = self.schema.unwrap_or_default();
        let format_version = self.format_version.unwrap_or(FormatVersion::V2);
        let location = self
            .location
            .ok_or_else(|| Error::InvalidMetadata("location required".into()))?;
        let table_uuid = self
            .table_uuid
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok(TableMetadata {
            format_version,
            table_uuid,
            location,
            last_sequence_number: 0,
            last_updated_ms: 0,
            last_column_id: schema.fields.iter().map(|f| f.id).max().unwrap_or(0),
            current_schema_id: schema.schema_id,
            schemas: vec![schema],
            partition_specs: vec![PartitionSpec::default()],
            default_spec_id: 0,
            last_partition_id: 999,
            sort_orders: vec![SortOrder::unsorted()],
            default_sort_order_id: 0,
            properties: HashMap::new(),
            current_snapshot_id: None,
            snapshots: vec![],
            snapshot_log: vec![],
            metadata_log: vec![],
            refs: HashMap::new(),
            next_row_id: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{NestedField, PrimitiveType, Type};

    #[test]
    fn format_version_serializes_as_int() {
        let j = serde_json::to_string(&FormatVersion::V2).unwrap();
        assert_eq!(j, "2");
    }

    #[test]
    fn unsupported_format_version_errors() {
        let r: Result<FormatVersion> = 9.try_into();
        assert!(matches!(r, Err(Error::UnsupportedFormatVersion(9))));
    }

    #[test]
    fn builder_requires_location() {
        let r = TableMetadataBuilder::new().build();
        assert!(matches!(r, Err(Error::InvalidMetadata(_))));
    }

    #[test]
    fn builder_assigns_last_column_id_from_schema() {
        let schema = Schema::builder()
            .with_field(NestedField::required(
                1,
                "a",
                Type::Primitive(PrimitiveType::Long),
            ))
            .with_field(NestedField::required(
                7,
                "b",
                Type::Primitive(PrimitiveType::Long),
            ))
            .build()
            .unwrap();
        let m = TableMetadataBuilder::new()
            .location("s3://x/t")
            .schema(schema)
            .build()
            .unwrap();
        assert_eq!(m.last_column_id, 7);
        assert_eq!(m.format_version, FormatVersion::V2);
    }

    #[test]
    fn current_schema_and_snapshot_lookup() {
        let schema = Schema::builder()
            .with_field(NestedField::required(
                1,
                "id",
                Type::Primitive(PrimitiveType::Long),
            ))
            .build()
            .unwrap();
        let m = TableMetadataBuilder::new()
            .location("s3://x/t")
            .schema(schema)
            .build()
            .unwrap();
        assert!(m.current_schema().is_some());
        assert!(m.current_snapshot().is_none());
    }

    #[test]
    fn metadata_json_round_trip() {
        let schema = Schema::builder()
            .with_field(NestedField::required(
                1,
                "id",
                Type::Primitive(PrimitiveType::Long),
            ))
            .build()
            .unwrap();
        let m = TableMetadataBuilder::new()
            .location("s3://x/t")
            .schema(schema)
            .build()
            .unwrap();
        let j = m.to_json().unwrap();
        let back = TableMetadata::from_json(&j).unwrap();
        assert_eq!(back.location, "s3://x/t");
        assert_eq!(back.format_version, FormatVersion::V2);
    }
}
