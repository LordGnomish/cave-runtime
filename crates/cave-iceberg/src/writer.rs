// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Data-file writer — `crates/iceberg/src/writer/`.
//!
//! The MVP writer captures the **control-plane** of an Iceberg write:
//! given a set of records (or row-deltas), it emits one or more
//! [`DataFile`] descriptors that the transaction coordinator will
//! attach to a new snapshot. Actual Parquet / Avro / ORC byte
//! generation is intentionally not implemented inside cave-iceberg —
//! the byte writer lives downstream in cave-runtime's storage layer
//! (or in a future `cave-parquet` crate). Per spec, only the data-file
//! *manifest entry* needs to come from the Iceberg layer for snapshot
//! isolation to work; the bytes are referenced by `file_path`.
//!
//! Supported write modes (matching upstream `WriteOperation`):
//!   * **append** — purely additive snapshot whose manifest entries
//!     are all `ManifestEntryStatus::Added`.
//!   * **overwrite** — replace specific rows: emit equality-deletes
//!     for the predicate, plus new data files for the replacement set.
//!   * **delete** — emit only equality-deletes (no new data files).

use std::collections::HashMap;

use crate::manifest::{
    DataFile, DataFileContent, FileFormat, ManifestEntry, ManifestEntryStatus,
};

/// Upstream `WriteOperation` enum. The summary string is written
/// verbatim into `Snapshot.summary["operation"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOperation {
    Append,
    Overwrite,
    Delete,
    Replace,
}

impl WriteOperation {
    pub fn as_str(&self) -> &'static str {
        match self {
            WriteOperation::Append => "append",
            WriteOperation::Overwrite => "overwrite",
            WriteOperation::Delete => "delete",
            WriteOperation::Replace => "replace",
        }
    }
}

/// Builder that accumulates one logical write before producing the
/// manifest entries the transaction will commit. The writer is
/// agnostic to row-binary encoding — callers pass a [`DataFile`]
/// directly, having already serialised their rows to the underlying
/// object-store path.
#[derive(Debug, Default)]
pub struct DataFileWriter {
    spec_id: i32,
    partition_values: HashMap<i32, serde_json::Value>,
    files: Vec<DataFile>,
}

impl DataFileWriter {
    pub fn new(spec_id: i32) -> Self {
        Self {
            spec_id,
            ..Default::default()
        }
    }

    /// Bind partition values for subsequent `add_file` calls.
    pub fn with_partition(mut self, partition: HashMap<i32, serde_json::Value>) -> Self {
        self.partition_values = partition;
        self
    }

    /// Append a data file. Returns `&mut self` for builder chaining.
    pub fn add_file(&mut self, file: DataFile) -> &mut Self {
        self.files.push(file);
        self
    }

    /// Convenience: build a [`DataFile`] for a flat Parquet path.
    pub fn append_parquet(
        &mut self,
        path: impl Into<String>,
        record_count: i64,
        file_size_bytes: i64,
    ) -> &mut Self {
        let mut f = DataFile::new(path.into(), FileFormat::Parquet);
        f.partition = self.partition_values.clone();
        f.record_count = record_count;
        f.file_size_in_bytes = file_size_bytes;
        self.files.push(f);
        self
    }

    /// Build the per-file [`ManifestEntry`]s for a *new* snapshot.
    pub fn build_manifest_entries(
        self,
        snapshot_id: i64,
        sequence_number: i64,
        status: ManifestEntryStatus,
    ) -> Vec<ManifestEntry> {
        self.files
            .into_iter()
            .map(|f| ManifestEntry {
                status,
                snapshot_id: Some(snapshot_id),
                sequence_number: Some(sequence_number),
                file_sequence_number: Some(sequence_number),
                data_file: f,
            })
            .collect()
    }

    pub fn spec_id(&self) -> i32 {
        self.spec_id
    }

    pub fn files_len(&self) -> usize {
        self.files.len()
    }
}

/// One overall write plan accumulated by a transaction. Captures the
/// operation type + the manifest entries to attach.
#[derive(Debug, Default)]
pub struct WritePlan {
    pub operation: Option<WriteOperation>,
    pub appended: Vec<ManifestEntry>,
    pub deleted: Vec<ManifestEntry>,
}

impl WritePlan {
    pub fn new(op: WriteOperation) -> Self {
        Self {
            operation: Some(op),
            appended: Vec::new(),
            deleted: Vec::new(),
        }
    }

    pub fn extend_appended(&mut self, entries: Vec<ManifestEntry>) -> &mut Self {
        self.appended.extend(entries);
        self
    }

    pub fn extend_deleted(&mut self, entries: Vec<ManifestEntry>) -> &mut Self {
        self.deleted.extend(entries);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.appended.is_empty() && self.deleted.is_empty()
    }

    /// Total added record count (positive) for the summary block.
    pub fn added_records(&self) -> i64 {
        self.appended
            .iter()
            .filter(|e| matches!(e.data_file.content, DataFileContent::Data))
            .map(|e| e.data_file.record_count)
            .sum()
    }

    /// Total deleted record count (positive) for the summary block.
    pub fn deleted_records(&self) -> i64 {
        self.deleted
            .iter()
            .filter(|e| {
                matches!(
                    e.data_file.content,
                    DataFileContent::EqualityDeletes | DataFileContent::PositionDeletes
                )
            })
            .map(|e| e.data_file.record_count)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_file(path: &str, records: i64) -> DataFile {
        let mut f = DataFile::new(path.to_string(), FileFormat::Parquet);
        f.content = DataFileContent::Data;
        f.record_count = records;
        f.file_size_in_bytes = records * 16;
        f
    }

    fn delete_file(path: &str, records: i64) -> DataFile {
        let mut f = DataFile::new(path.to_string(), FileFormat::Parquet);
        f.content = DataFileContent::EqualityDeletes;
        f.record_count = records;
        f.file_size_in_bytes = records * 8;
        f
    }

    #[test]
    fn data_file_writer_collects_files_with_partition() {
        let mut p = HashMap::new();
        p.insert(1, serde_json::json!(2026));
        let mut w = DataFileWriter::new(0).with_partition(p);
        w.append_parquet("data/x.parquet", 100, 4096);
        assert_eq!(w.files_len(), 1);
        let entries = w.build_manifest_entries(1, 1, ManifestEntryStatus::Added);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, ManifestEntryStatus::Added);
        assert_eq!(entries[0].snapshot_id, Some(1));
        assert_eq!(entries[0].data_file.record_count, 100);
        assert_eq!(
            entries[0].data_file.partition.get(&1),
            Some(&serde_json::json!(2026))
        );
    }

    #[test]
    fn write_plan_append_aggregates_added_records() {
        let mut w = DataFileWriter::new(0);
        w.add_file(data_file("data/a.parquet", 10));
        w.add_file(data_file("data/b.parquet", 20));
        let entries = w.build_manifest_entries(1, 1, ManifestEntryStatus::Added);
        let mut plan = WritePlan::new(WriteOperation::Append);
        plan.extend_appended(entries);
        assert_eq!(plan.added_records(), 30);
        assert_eq!(plan.deleted_records(), 0);
        assert_eq!(plan.appended.len(), 2);
    }

    #[test]
    fn write_plan_delete_records_equality_deletes() {
        let mut w = DataFileWriter::new(0);
        w.add_file(delete_file("deletes/eq-1.parquet", 5));
        let entries = w.build_manifest_entries(2, 2, ManifestEntryStatus::Deleted);
        let mut plan = WritePlan::new(WriteOperation::Delete);
        plan.extend_deleted(entries);
        assert_eq!(plan.deleted_records(), 5);
        assert_eq!(plan.added_records(), 0);
        assert_eq!(plan.operation.unwrap().as_str(), "delete");
    }

    #[test]
    fn write_plan_is_empty_until_extended() {
        let plan = WritePlan::new(WriteOperation::Append);
        assert!(plan.is_empty());
        let mut w = DataFileWriter::new(0);
        w.add_file(data_file("a.parquet", 1));
        let entries = w.build_manifest_entries(1, 1, ManifestEntryStatus::Added);
        let mut plan = WritePlan::new(WriteOperation::Append);
        plan.extend_appended(entries);
        assert!(!plan.is_empty());
    }

    #[test]
    fn write_operation_str_matches_iceberg_spec() {
        assert_eq!(WriteOperation::Append.as_str(), "append");
        assert_eq!(WriteOperation::Overwrite.as_str(), "overwrite");
        assert_eq!(WriteOperation::Delete.as_str(), "delete");
        assert_eq!(WriteOperation::Replace.as_str(), "replace");
    }

    #[test]
    fn data_file_writer_records_spec_id() {
        let w = DataFileWriter::new(3);
        assert_eq!(w.spec_id(), 3);
    }

    #[test]
    fn write_plan_handles_overwrite_mix() {
        // Overwrite: equality deletes for the matched rows + new
        // data files with the replacement rows.
        let mut p = WritePlan::new(WriteOperation::Overwrite);

        let mut del = DataFileWriter::new(0);
        del.add_file(delete_file("deletes/o-1.parquet", 7));
        p.extend_deleted(del.build_manifest_entries(3, 3, ManifestEntryStatus::Deleted));

        let mut add = DataFileWriter::new(0);
        add.add_file(data_file("data/o-1.parquet", 11));
        p.extend_appended(add.build_manifest_entries(3, 3, ManifestEntryStatus::Added));

        assert_eq!(p.added_records(), 11);
        assert_eq!(p.deleted_records(), 7);
        assert_eq!(p.operation.unwrap(), WriteOperation::Overwrite);
    }

    #[test]
    fn append_parquet_uses_data_file_content_data() {
        let mut w = DataFileWriter::new(0);
        w.append_parquet("a.parquet", 42, 1024);
        let entries = w.build_manifest_entries(1, 1, ManifestEntryStatus::Added);
        assert!(matches!(
            entries[0].data_file.content,
            DataFileContent::Data
        ));
        assert!(matches!(
            entries[0].data_file.file_format,
            FileFormat::Parquet
        ));
    }
}
