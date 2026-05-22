// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg scan planning.
//!
//! Upstream:
//! * `crates/iceberg/src/scan.rs`
//!
//! `ScanBuilder` is the entry point a query engine uses to plan a
//! read: pick a snapshot, attach a predicate, optionally select columns,
//! and yield a list of `FileScanTask` rows. The MVP plans against the
//! in-memory metadata; it returns one task per live data file from the
//! current snapshot's manifest chain. Wiring to an actual manifest
//! reader (Avro) lives behind a `[[scope_cuts]] avro-wire` entry —
//! today the planner expects manifests to be supplied externally via
//! `add_manifest()`.

use crate::manifest::Manifest;
use crate::table_metadata::TableMetadata;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileScanTask {
    pub data_file_path: String,
    pub record_count: i64,
    pub file_size_in_bytes: i64,
    pub start: i64,
    pub length: i64,
    pub schema_id: i32,
    pub partition_spec_id: i32,
    /// The serialized predicate that the engine should re-evaluate
    /// against the file's stats / row groups.
    pub residual_filter_json: Option<String>,
}

#[derive(Debug)]
pub struct ScanBuilder {
    metadata: Arc<TableMetadata>,
    selected_columns: Option<Vec<String>>,
    predicate: Option<crate::expr::Predicate>,
    snapshot_id: Option<i64>,
    manifests: Vec<Manifest>,
}

impl ScanBuilder {
    pub fn new(metadata: Arc<TableMetadata>) -> Self {
        Self {
            metadata,
            selected_columns: None,
            predicate: None,
            snapshot_id: None,
            manifests: Vec::new(),
        }
    }

    pub fn select<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.selected_columns = Some(cols.into_iter().map(Into::into).collect());
        self
    }

    pub fn filter(mut self, p: crate::expr::Predicate) -> Self {
        self.predicate = Some(p);
        self
    }

    pub fn snapshot(mut self, id: i64) -> Self {
        self.snapshot_id = Some(id);
        self
    }

    /// Attach a manifest reader's already-decoded contents. The actual
    /// Avro reader for manifests is deferred (`[[scope_cuts]] avro-wire`);
    /// engines wire the contents externally for now.
    pub fn add_manifest(mut self, m: Manifest) -> Self {
        self.manifests.push(m);
        self
    }

    pub fn snapshot_id(&self) -> Option<i64> {
        self.snapshot_id
            .or_else(|| self.metadata.current_snapshot_id)
    }

    pub fn selected_columns(&self) -> Option<&[String]> {
        self.selected_columns.as_deref()
    }

    /// Plan the scan: walk attached manifests, emit one FileScanTask
    /// per live data file. The residual predicate (rows the engine
    /// still needs to filter post-read) is attached verbatim — there
    /// is no manifest-time partition-bound pruning in the MVP.
    pub fn plan_files(&self) -> Vec<FileScanTask> {
        let residual = self
            .predicate
            .as_ref()
            .and_then(|p| serde_json::to_string(p).ok());
        let schema_id = self.metadata.current_schema_id;
        let default_spec = self.metadata.default_spec_id;

        let mut out = Vec::new();
        for m in &self.manifests {
            for entry in m.live_entries() {
                if entry.data_file.content != crate::manifest::DataFileContent::Data {
                    continue;
                }
                let df = &entry.data_file;
                out.push(FileScanTask {
                    data_file_path: df.file_path.clone(),
                    record_count: df.record_count,
                    file_size_in_bytes: df.file_size_in_bytes,
                    start: 0,
                    length: df.file_size_in_bytes,
                    schema_id,
                    partition_spec_id: default_spec,
                    residual_filter_json: residual.clone(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{DataFile, FileFormat, ManifestEntry};
    use crate::schema::Schema;
    use crate::table_metadata::TableMetadataBuilder;

    fn meta() -> Arc<TableMetadata> {
        Arc::new(
            TableMetadataBuilder::new()
                .location("s3://x/t")
                .schema(Schema::default())
                .build()
                .unwrap(),
        )
    }

    #[test]
    fn plan_files_emits_one_per_live_data_file() {
        let mut df1 = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
        df1.record_count = 100;
        df1.file_size_in_bytes = 1024;
        let mut df2 = DataFile::new("s3://x/b.parquet", FileFormat::Parquet);
        df2.record_count = 200;
        df2.file_size_in_bytes = 2048;
        let m = Manifest {
            partition_spec_id: 0,
            schema_id: 0,
            entries: vec![
                ManifestEntry::added(1, df1),
                ManifestEntry::added(1, df2),
            ],
        };
        let tasks = ScanBuilder::new(meta()).add_manifest(m).plan_files();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].record_count, 100);
        assert_eq!(tasks[1].length, 2048);
    }

    #[test]
    fn plan_files_skips_deleted_entries() {
        let df = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
        let m = Manifest {
            partition_spec_id: 0,
            schema_id: 0,
            entries: vec![ManifestEntry::deleted(1, df)],
        };
        let tasks = ScanBuilder::new(meta()).add_manifest(m).plan_files();
        assert!(tasks.is_empty());
    }

    #[test]
    fn select_records_columns() {
        let b = ScanBuilder::new(meta()).select(["a", "b"]);
        assert_eq!(
            b.selected_columns(),
            Some(&["a".to_string(), "b".to_string()][..])
        );
    }

    #[test]
    fn filter_attaches_residual_predicate_to_each_task() {
        let mut df = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
        df.record_count = 5;
        let m = Manifest {
            partition_spec_id: 0,
            schema_id: 0,
            entries: vec![ManifestEntry::added(1, df)],
        };
        let tasks = ScanBuilder::new(meta())
            .filter(crate::expr::Predicate::eq(
                crate::expr::Term::ref_col("a"),
                crate::expr::Term::lit(1),
            ))
            .add_manifest(m)
            .plan_files();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].residual_filter_json.is_some());
    }

    #[test]
    fn snapshot_id_pins_explicitly() {
        let b = ScanBuilder::new(meta()).snapshot(42);
        assert_eq!(b.snapshot_id(), Some(42));
    }
}
