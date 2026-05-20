// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg Manifest file model.
//!
//! Upstream:
//! * `crates/iceberg/src/spec/manifest.rs`
//! Spec: <https://iceberg.apache.org/spec/#manifests>
//!
//! A manifest is the per-snapshot inventory of data files (or delete
//! files). It lists each file with its path, format, partition values,
//! upper/lower bounds, file size, record count, equality/positional
//! delete flags, sequence number, and snapshot id. Stored in Avro by
//! Iceberg; the MVP holds the in-memory form and can round-trip JSON.
//! Avro encoding is deferred — see `[[scope_cuts]] avro-wire`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FileFormat {
    Parquet,
    Avro,
    Orc,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DataFileContent {
    Data,
    PositionDeletes,
    EqualityDeletes,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ManifestEntryStatus {
    Existing = 0,
    Added = 1,
    Deleted = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataFile {
    pub content: DataFileContent,
    pub file_path: String,
    pub file_format: FileFormat,
    /// Iceberg's "partition tuple" — by-field-id JSON values.
    #[serde(default)]
    pub partition: HashMap<i32, serde_json::Value>,
    pub record_count: i64,
    pub file_size_in_bytes: i64,
    #[serde(default)]
    pub column_sizes: HashMap<i32, i64>,
    #[serde(default)]
    pub value_counts: HashMap<i32, i64>,
    #[serde(default)]
    pub null_value_counts: HashMap<i32, i64>,
    #[serde(default)]
    pub nan_value_counts: HashMap<i32, i64>,
    /// Per-column lower bound — serialized form (raw bytes encoded as
    /// hex strings in the JSON wire).
    #[serde(default)]
    pub lower_bounds: HashMap<i32, String>,
    #[serde(default)]
    pub upper_bounds: HashMap<i32, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_metadata: Option<String>,
    #[serde(default)]
    pub split_offsets: Vec<i64>,
    /// Sort-order-id this file was written under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_order_id: Option<i32>,
    /// Iceberg's "equality-ids" for equality-delete files.
    #[serde(default)]
    pub equality_ids: Vec<i32>,
}

impl DataFile {
    pub fn new(file_path: impl Into<String>, file_format: FileFormat) -> Self {
        Self {
            content: DataFileContent::Data,
            file_path: file_path.into(),
            file_format,
            partition: HashMap::new(),
            record_count: 0,
            file_size_in_bytes: 0,
            column_sizes: HashMap::new(),
            value_counts: HashMap::new(),
            null_value_counts: HashMap::new(),
            nan_value_counts: HashMap::new(),
            lower_bounds: HashMap::new(),
            upper_bounds: HashMap::new(),
            key_metadata: None,
            split_offsets: Vec::new(),
            sort_order_id: None,
            equality_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestEntry {
    pub status: ManifestEntryStatus,
    pub snapshot_id: Option<i64>,
    pub sequence_number: Option<i64>,
    pub file_sequence_number: Option<i64>,
    pub data_file: DataFile,
}

impl ManifestEntry {
    pub fn added(snapshot_id: i64, data_file: DataFile) -> Self {
        Self {
            status: ManifestEntryStatus::Added,
            snapshot_id: Some(snapshot_id),
            sequence_number: Some(snapshot_id),
            file_sequence_number: Some(snapshot_id),
            data_file,
        }
    }

    pub fn existing(snapshot_id: i64, data_file: DataFile) -> Self {
        Self {
            status: ManifestEntryStatus::Existing,
            snapshot_id: Some(snapshot_id),
            sequence_number: Some(snapshot_id),
            file_sequence_number: Some(snapshot_id),
            data_file,
        }
    }

    pub fn deleted(snapshot_id: i64, data_file: DataFile) -> Self {
        Self {
            status: ManifestEntryStatus::Deleted,
            snapshot_id: Some(snapshot_id),
            sequence_number: Some(snapshot_id),
            file_sequence_number: Some(snapshot_id),
            data_file,
        }
    }

    pub fn is_live(&self) -> bool {
        matches!(
            self.status,
            ManifestEntryStatus::Added | ManifestEntryStatus::Existing
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Manifest {
    pub partition_spec_id: i32,
    pub schema_id: i32,
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub fn live_entries(&self) -> impl Iterator<Item = &ManifestEntry> {
        self.entries.iter().filter(|e| e.is_live())
    }

    pub fn live_data_files(&self) -> impl Iterator<Item = &DataFile> {
        self.live_entries()
            .filter(|e| e.data_file.content == DataFileContent::Data)
            .map(|e| &e.data_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_entry_lifecycle() {
        let f = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
        let added = ManifestEntry::added(1, f.clone());
        let deleted = ManifestEntry::deleted(2, f.clone());
        assert!(added.is_live());
        assert!(!deleted.is_live());
    }

    #[test]
    fn live_data_files_excludes_deletes_and_delete_files() {
        let mut data = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
        data.record_count = 100;
        let mut delete_file = DataFile::new("s3://x/d.parquet", FileFormat::Parquet);
        delete_file.content = DataFileContent::EqualityDeletes;

        let m = Manifest {
            partition_spec_id: 0,
            schema_id: 0,
            entries: vec![
                ManifestEntry::added(1, data.clone()),
                ManifestEntry::deleted(2, data.clone()),
                ManifestEntry::added(3, delete_file),
            ],
        };
        let live: Vec<_> = m.live_data_files().collect();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].file_path, "s3://x/a.parquet");
    }

    #[test]
    fn data_file_serializes_kebab_format() {
        let f = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
        let j = serde_json::to_value(&f).unwrap();
        assert_eq!(j["file_format"], "parquet");
        assert_eq!(j["content"], "data");
    }
}
