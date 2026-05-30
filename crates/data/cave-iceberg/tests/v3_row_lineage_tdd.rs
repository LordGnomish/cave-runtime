// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD — Iceberg v3 **row lineage**.
//!
//! Upstream spec (format/spec.md, v3 additions):
//!   * Table metadata gains `next-row-id` — the row id to assign to the
//!     next added row.
//!   * Each snapshot gains `first-row-id` (the `_row_id` of the first row
//!     in its first data file) and `added-rows` (upper bound of rows that
//!     received assigned row ids).
//!   * On commit the snapshot's `first-row-id` is set to the table's
//!     current `next-row-id`, then `next-row-id += added-rows`.
//!   * A data file inherits row ids: file `i` starts at
//!     `snapshot.first_row_id + sum(record_counts[0..i])`, and a row's
//!     `_row_id` is `file.first_row_id + _pos`.
//!
//! Closes the v3-spec partial (row-id assignment) in parity.manifest.toml.

use cave_iceberg::snapshot::assign_data_file_first_row_ids;
use cave_iceberg::{Schema, Snapshot, TableMetadata};
use std::collections::HashMap;

fn base_metadata() -> TableMetadata {
    TableMetadata::builder()
        .location("s3://wh/db.tbl")
        .schema(Schema::default())
        .build()
        .unwrap()
}

fn bare_snapshot(id: i64, parent: Option<i64>) -> Snapshot {
    Snapshot {
        snapshot_id: id,
        parent_snapshot_id: parent,
        sequence_number: id,
        timestamp_ms: 0,
        manifest_list: format!("s3://wh/snap-{id}.avro"),
        summary: HashMap::new(),
        schema_id: Some(0),
        first_row_id: None,
        added_rows: None,
    }
}

#[test]
fn next_row_id_defaults_to_zero() {
    let m = base_metadata();
    assert_eq!(m.next_row_id, 0);
}

#[test]
fn assign_sets_first_row_id_and_advances_next() {
    let mut m = base_metadata();
    let mut s = bare_snapshot(1, None);

    // First commit adds 100 rows.
    m.assign_snapshot_row_ids(&mut s, 100);
    assert_eq!(s.first_row_id, Some(0));
    assert_eq!(s.added_rows, Some(100));
    assert_eq!(m.next_row_id, 100, "next-row-id advances by added-rows");
}

#[test]
fn assign_is_monotonic_across_snapshots() {
    let mut m = base_metadata();

    let mut s1 = bare_snapshot(1, None);
    m.assign_snapshot_row_ids(&mut s1, 100);

    let mut s2 = bare_snapshot(2, Some(1));
    m.assign_snapshot_row_ids(&mut s2, 50);

    assert_eq!(s1.first_row_id, Some(0));
    assert_eq!(s2.first_row_id, Some(100), "second snapshot starts where first ended");
    assert_eq!(s2.added_rows, Some(50));
    assert_eq!(m.next_row_id, 150);
}

#[test]
fn empty_commit_does_not_advance_next_row_id() {
    let mut m = base_metadata();
    let mut s = bare_snapshot(1, None);
    m.assign_snapshot_row_ids(&mut s, 0);
    assert_eq!(s.first_row_id, Some(0));
    assert_eq!(s.added_rows, Some(0));
    assert_eq!(m.next_row_id, 0);
}

#[test]
fn data_files_inherit_cumulative_first_row_ids() {
    // Snapshot first_row_id = 1000, three data files of 10/20/5 rows.
    let ids = assign_data_file_first_row_ids(1000, &[10, 20, 5]);
    assert_eq!(ids, vec![1000, 1010, 1030]);
}

#[test]
fn data_file_first_row_id_empty_input() {
    let ids = assign_data_file_first_row_ids(42, &[]);
    assert!(ids.is_empty());
}

#[test]
fn snapshot_row_lineage_round_trips_kebab_json() {
    let mut m = base_metadata();
    let mut s = bare_snapshot(7, None);
    m.assign_snapshot_row_ids(&mut s, 99);

    let j = serde_json::to_value(&s).unwrap();
    assert_eq!(j["first-row-id"], 0);
    assert_eq!(j["added-rows"], 99);

    let back: Snapshot = serde_json::from_value(j).unwrap();
    assert_eq!(back.first_row_id, Some(0));
    assert_eq!(back.added_rows, Some(99));
}

#[test]
fn next_row_id_serializes_in_metadata_json() {
    let mut m = base_metadata();
    let mut s = bare_snapshot(1, None);
    m.assign_snapshot_row_ids(&mut s, 256);
    let j = serde_json::to_string(&m).unwrap();
    assert!(j.contains("\"next-row-id\":256"), "metadata JSON must carry next-row-id, got: {j}");
}
