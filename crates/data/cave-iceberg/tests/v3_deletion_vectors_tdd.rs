// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD — Iceberg v3 **deletion vectors**.
//!
//! Upstream spec (format/spec.md, v3 additions):
//!   * Position deletes may be stored as a deletion vector in the Puffin
//!     format (file_format = "puffin", content = position-deletes).
//!   * The delete file carries `referenced_data_file` (143, the data file
//!     all deletes target), `content_offset` (144, byte offset where the
//!     vector blob starts in the Puffin file) and `content_size_in_bytes`
//!     (145, blob length).
//!   * A deletion vector references exactly one data file.
//!
//! Closes the v3-spec partial (deletion-vector references) in
//! parity.manifest.toml.

use cave_iceberg::{DataFile, DataFileContent, FileFormat};

#[test]
fn puffin_file_format_serializes_kebab() {
    let v = serde_json::to_value(FileFormat::Puffin).unwrap();
    assert_eq!(v, serde_json::json!("puffin"));
}

#[test]
fn data_file_dv_fields_default_none() {
    let f = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
    assert!(f.referenced_data_file.is_none());
    assert!(f.content_offset.is_none());
    assert!(f.content_size_in_bytes.is_none());
}

#[test]
fn deletion_vector_ctor_sets_puffin_position_deletes() {
    let dv = DataFile::deletion_vector(
        "s3://x/deletes/dv-1.puffin",
        "s3://x/data/a.parquet",
        4,
        128,
    );
    assert_eq!(dv.content, DataFileContent::PositionDeletes);
    assert_eq!(dv.file_format, FileFormat::Puffin);
    assert_eq!(dv.file_path, "s3://x/deletes/dv-1.puffin");
    assert_eq!(dv.referenced_data_file.as_deref(), Some("s3://x/data/a.parquet"));
    assert_eq!(dv.content_offset, Some(4));
    assert_eq!(dv.content_size_in_bytes, Some(128));
}

#[test]
fn is_deletion_vector_predicate() {
    let dv = DataFile::deletion_vector("s3://x/dv.puffin", "s3://x/a.parquet", 0, 16);
    assert!(dv.is_deletion_vector());

    // A plain data file is not a deletion vector.
    let data = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
    assert!(!data.is_deletion_vector());

    // A parquet positional-delete file (legacy, not a DV) is not a DV.
    let mut legacy = DataFile::new("s3://x/pos.parquet", FileFormat::Parquet);
    legacy.content = DataFileContent::PositionDeletes;
    assert!(!legacy.is_deletion_vector());
}

#[test]
fn deletion_vector_round_trips_json() {
    let dv = DataFile::deletion_vector("s3://x/dv.puffin", "s3://x/a.parquet", 4, 128);
    let v = serde_json::to_value(&dv).unwrap();
    assert_eq!(v["file_format"], "puffin");
    assert_eq!(v["content"], "position-deletes");
    assert_eq!(v["referenced_data_file"], "s3://x/a.parquet");
    assert_eq!(v["content_offset"], 4);
    assert_eq!(v["content_size_in_bytes"], 128);

    let back: DataFile = serde_json::from_value(v).unwrap();
    assert_eq!(back, dv);
}

#[test]
fn dv_fields_omitted_when_unset() {
    let f = DataFile::new("s3://x/a.parquet", FileFormat::Parquet);
    let s = serde_json::to_string(&f).unwrap();
    assert!(!s.contains("referenced_data_file"));
    assert!(!s.contains("content_offset"));
    assert!(!s.contains("content_size_in_bytes"));
}
