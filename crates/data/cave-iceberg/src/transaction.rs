// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Transaction commit coordinator — `crates/iceberg/src/transaction.rs`.
//!
//! A transaction is the unit of metadata change: it captures a base
//! `TableMetadata`, layers one or more pending [`WritePlan`]s, and
//! produces a new `TableMetadata` whose `current_snapshot_id` points
//! at a freshly-minted [`Snapshot`]. The commit primitive is delegated
//! to the underlying [`Catalog::replace_table_metadata`] — the cave
//! transaction coordinator does **not** assume a CAS object-store
//! (object-store CAS is a v0.2 milestone tracked under
//! `lakehouse-ray-2`).
//!
//! The coordinator implements three commit operations matching
//! upstream:
//!
//! * `append_files`        — purely additive snapshot.
//! * `overwrite_files`     — overwrite-by-equality-predicate snapshot.
//! * `delete_data_files`   — delete-only snapshot.

use crate::manifest::{Manifest, ManifestEntry, ManifestEntryStatus};
use crate::snapshot::Snapshot;
use crate::table_metadata::TableMetadata;
use crate::writer::{WriteOperation, WritePlan};
use std::collections::HashMap;

/// A pending transaction layered on a base `TableMetadata`.
///
/// The transaction is non-async + lock-free; threading is the caller's
/// responsibility. The base metadata is owned by the transaction so
/// that the commit step is a pure function of (base, plan).
pub struct Transaction {
    base: TableMetadata,
    plans: Vec<WritePlan>,
    /// Auto-incrementing snapshot id source. Initialised from
    /// `base.current_snapshot_id.unwrap_or(0)` + 1; bumped per commit.
    next_snapshot_id: i64,
}

impl Transaction {
    pub fn new(base: TableMetadata) -> Self {
        let next = base.current_snapshot_id.unwrap_or(0) + 1;
        Self {
            base,
            plans: Vec::new(),
            next_snapshot_id: next,
        }
    }

    /// Layer an additive write — equivalent to upstream's
    /// `AppendFiles.commit`.
    pub fn append_files(&mut self, plan: WritePlan) -> &mut Self {
        debug_assert_eq!(plan.operation, Some(WriteOperation::Append));
        self.plans.push(plan);
        self
    }

    /// Layer an overwrite plan — equivalent to
    /// `OverwriteFiles.commit`.
    pub fn overwrite_files(&mut self, plan: WritePlan) -> &mut Self {
        debug_assert_eq!(plan.operation, Some(WriteOperation::Overwrite));
        self.plans.push(plan);
        self
    }

    /// Layer a delete plan — equivalent to `DeleteFiles.commit`.
    pub fn delete_data_files(&mut self, plan: WritePlan) -> &mut Self {
        debug_assert_eq!(plan.operation, Some(WriteOperation::Delete));
        self.plans.push(plan);
        self
    }

    /// Number of pending plans.
    pub fn pending(&self) -> usize {
        self.plans.len()
    }

    /// Produce the new `TableMetadata` *without* persisting it.
    /// Composable when callers want to inspect the proposed snapshot
    /// before handing it to a Catalog.
    pub fn build_metadata(self, now_ms: i64) -> TableMetadata {
        let mut new_meta = self.base.clone();
        let mut snap_id = self.next_snapshot_id;
        let parent = self.base.current_snapshot_id;
        let mut last_parent = parent;
        let mut sequence_number = self.base.last_sequence_number;

        for plan in self.plans {
            if plan.is_empty() {
                continue;
            }
            sequence_number += 1;
            let added_records = plan.added_records();
            let op = plan
                .operation
                .as_ref()
                .map(|o| o.as_str())
                .unwrap_or("append");
            let mut summary: HashMap<String, String> = HashMap::new();
            summary.insert("operation".into(), op.into());
            summary.insert("added-records".into(), plan.added_records().to_string());
            summary.insert("deleted-records".into(), plan.deleted_records().to_string());
            summary.insert(
                "added-data-files".into(),
                plan.appended.len().to_string(),
            );
            summary.insert(
                "deleted-data-files".into(),
                plan.deleted.len().to_string(),
            );

            let manifest_list = format!(
                "{}/metadata/snap-{}-{}-{}.avro",
                new_meta.location, snap_id, sequence_number, op
            );

            let mut snap = Snapshot {
                snapshot_id: snap_id,
                parent_snapshot_id: last_parent,
                sequence_number,
                timestamp_ms: now_ms,
                manifest_list,
                summary,
                schema_id: Some(new_meta.current_schema_id),
                first_row_id: None,
                added_rows: None,
            };

            // Iceberg v3 row lineage — stamp the committing snapshot with a
            // contiguous `_row_id` range and advance the table's next-row-id.
            if new_meta.format_version == crate::table_metadata::FormatVersion::V3 {
                new_meta.assign_snapshot_row_ids(&mut snap, added_records);
            }

            new_meta.snapshots.push(snap);
            new_meta.current_snapshot_id = Some(snap_id);
            new_meta.last_sequence_number = sequence_number;
            new_meta.last_updated_ms = now_ms;
            new_meta.snapshot_log.push(crate::table_metadata::SnapshotLogEntry {
                snapshot_id: snap_id,
                timestamp_ms: now_ms,
            });

            last_parent = Some(snap_id);
            snap_id += 1;
        }

        new_meta
    }
}

/// Roll all live (Added + Existing) entries from a base manifest into
/// a new manifest, applying the deletions implied by `plan.deleted`
/// (matched by `file_path`). The result is the post-commit manifest a
/// catalog would persist alongside the new `TableMetadata`.
pub fn rewrite_manifest(base: &Manifest, plan: &WritePlan) -> Manifest {
    let deleted_paths: std::collections::HashSet<&str> = plan
        .deleted
        .iter()
        .map(|e| e.data_file.file_path.as_str())
        .collect();

    let mut entries: Vec<ManifestEntry> = base
        .entries
        .iter()
        .filter(|e| e.is_live())
        .filter(|e| !deleted_paths.contains(e.data_file.file_path.as_str()))
        .map(|e| {
            let mut e2 = e.clone();
            e2.status = ManifestEntryStatus::Existing;
            e2
        })
        .collect();
    entries.extend(plan.appended.iter().cloned());

    Manifest {
        partition_spec_id: base.partition_spec_id,
        schema_id: base.schema_id,
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{DataFile, DataFileContent, FileFormat};
    use crate::writer::DataFileWriter;

    fn make_metadata() -> TableMetadata {
        TableMetadata::builder()
            .format_version(crate::table_metadata::FormatVersion::V2)
            .location("s3://bucket/tbl".to_string())
            .build()
            .unwrap()
    }

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

    fn build_append_plan(files: Vec<DataFile>) -> WritePlan {
        let mut w = DataFileWriter::new(0);
        for f in files {
            w.add_file(f);
        }
        let entries = w.build_manifest_entries(0, 0, ManifestEntryStatus::Added);
        let mut plan = WritePlan::new(WriteOperation::Append);
        plan.extend_appended(entries);
        plan
    }

    fn build_delete_plan(files: Vec<DataFile>) -> WritePlan {
        let mut w = DataFileWriter::new(0);
        for f in files {
            w.add_file(f);
        }
        let entries = w.build_manifest_entries(0, 0, ManifestEntryStatus::Deleted);
        let mut plan = WritePlan::new(WriteOperation::Delete);
        plan.extend_deleted(entries);
        plan
    }

    #[test]
    fn append_commits_a_new_snapshot() {
        let base = make_metadata();
        let mut tx = Transaction::new(base);
        let plan = build_append_plan(vec![data_file("data/a.parquet", 10)]);
        tx.append_files(plan);
        let after = tx.build_metadata(1_700_000_000_000);
        assert!(after.current_snapshot_id.is_some());
        assert_eq!(after.snapshots.len(), 1);
        let snap = after.current_snapshot().unwrap();
        assert_eq!(snap.summary.get("operation"), Some(&"append".to_string()));
        assert_eq!(snap.summary.get("added-records"), Some(&"10".to_string()));
        assert_eq!(after.last_sequence_number, 1);
        assert_eq!(after.snapshot_log.len(), 1);
    }

    #[test]
    fn empty_plan_does_not_advance_snapshot() {
        let base = make_metadata();
        let mut tx = Transaction::new(base);
        let empty = WritePlan::new(WriteOperation::Append);
        tx.append_files(empty);
        let after = tx.build_metadata(100);
        assert!(after.current_snapshot_id.is_none());
        assert!(after.snapshots.is_empty());
        assert_eq!(after.last_sequence_number, 0);
    }

    #[test]
    fn overwrite_records_both_added_and_deleted() {
        let base = make_metadata();
        let mut tx = Transaction::new(base);
        let mut plan = WritePlan::new(WriteOperation::Overwrite);
        plan.extend_appended(
            DataFileWriter::new(0)
                .add_file_chain(data_file("data/o-1.parquet", 11))
                .build_manifest_entries(0, 0, ManifestEntryStatus::Added),
        );
        plan.extend_deleted(
            DataFileWriter::new(0)
                .add_file_chain(delete_file("deletes/o-1.parquet", 7))
                .build_manifest_entries(0, 0, ManifestEntryStatus::Deleted),
        );
        tx.overwrite_files(plan);
        let after = tx.build_metadata(100);
        let snap = after.current_snapshot().unwrap();
        assert_eq!(snap.summary.get("operation"), Some(&"overwrite".to_string()));
        assert_eq!(snap.summary.get("added-records"), Some(&"11".to_string()));
        assert_eq!(snap.summary.get("deleted-records"), Some(&"7".to_string()));
    }

    #[test]
    fn multiple_plans_produce_chained_snapshots() {
        let base = make_metadata();
        let mut tx = Transaction::new(base);
        tx.append_files(build_append_plan(vec![data_file("a.parquet", 5)]));
        tx.append_files(build_append_plan(vec![data_file("b.parquet", 10)]));
        let after = tx.build_metadata(100);
        assert_eq!(after.snapshots.len(), 2);
        // 2nd snapshot's parent must point at 1st.
        assert_eq!(
            after.snapshots[1].parent_snapshot_id,
            Some(after.snapshots[0].snapshot_id)
        );
        assert_eq!(after.last_sequence_number, 2);
    }

    #[test]
    fn rewrite_manifest_drops_deleted_paths_and_appends_new() {
        // Base manifest has 3 live entries; deletion plan targets 1.
        let live: Vec<ManifestEntry> = vec![
            ManifestEntry::existing(1, data_file("data/a.parquet", 10)),
            ManifestEntry::existing(1, data_file("data/b.parquet", 20)),
            ManifestEntry::existing(1, data_file("data/c.parquet", 30)),
        ];
        let base = Manifest {
            partition_spec_id: 0,
            schema_id: 0,
            entries: live,
        };
        let plan = build_delete_plan(vec![delete_file("data/b.parquet", 1)]);
        // Note: rewrite uses file_path; the equality-delete entry's
        // file_path here doubles as the target path for the test.
        let after = rewrite_manifest(&base, &plan);
        assert_eq!(after.entries.len(), 2);
        assert!(after
            .entries
            .iter()
            .all(|e| e.data_file.file_path != "data/b.parquet"));
        assert!(after.entries.iter().all(|e| e.is_live()));
    }

    #[test]
    fn delete_only_plan_advances_snapshot_with_zero_added() {
        let base = make_metadata();
        let mut tx = Transaction::new(base);
        tx.delete_data_files(build_delete_plan(vec![delete_file("d-1.parquet", 4)]));
        let after = tx.build_metadata(100);
        let snap = after.current_snapshot().unwrap();
        assert_eq!(snap.summary.get("operation"), Some(&"delete".to_string()));
        assert_eq!(snap.summary.get("added-records"), Some(&"0".to_string()));
        assert_eq!(snap.summary.get("deleted-records"), Some(&"4".to_string()));
    }

    #[test]
    fn transaction_with_no_plans_is_no_op() {
        let base = make_metadata();
        let tx = Transaction::new(base.clone());
        assert_eq!(tx.pending(), 0);
        let after = tx.build_metadata(1);
        assert_eq!(after.snapshots.len(), 0);
        assert_eq!(after.current_snapshot_id, base.current_snapshot_id);
    }

    // Tiny helper so we can chain `add_file` in the overwrite test.
    impl crate::writer::DataFileWriter {
        pub fn add_file_chain(mut self, f: DataFile) -> Self {
            self.add_file(f);
            self
        }
    }
}
