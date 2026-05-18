// SPDX-License-Identifier: AGPL-3.0-or-later
//! Iceberg Manifest — list of data files belonging to one snapshot.
//!
//! Mirrors apache/iceberg-rust crates/iceberg/src/spec/manifest.rs and
//! the spec at https://iceberg.apache.org/spec/#manifests.

use crate::table_format::iceberg::error::{IcebergError, IcebergResult};
use crate::table_format::iceberg::tenant::{default_tenant_id, validate_tenant_id};
use serde::{Deserialize, Serialize};

/// File format of a data file referenced by the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DataFileFormat {
    Parquet,
    Avro,
    Orc,
}

/// CRUD lifecycle status of a manifest entry — matches iceberg spec
/// `ManifestEntry.status` enum (0=EXISTING, 1=ADDED, 2=DELETED).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManifestEntryStatus {
    Existing,
    Added,
    Deleted,
}

impl ManifestEntryStatus {
    /// Numeric value used in the Avro-encoded manifest file
    /// (apache/iceberg-rust spec/manifest.rs).
    pub const fn code(self) -> i32 {
        match self {
            ManifestEntryStatus::Existing => 0,
            ManifestEntryStatus::Added => 1,
            ManifestEntryStatus::Deleted => 2,
        }
    }
}

/// `content` flavor of the data file — DATA / POSITION_DELETES / EQUALITY_DELETES.
/// Iceberg v2 spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataContent {
    Data,
    PositionDeletes,
    EqualityDeletes,
}

impl DataContent {
    pub const fn code(self) -> i32 {
        // citation: iceberg spec v2 — content codes 0/1/2
        match self {
            DataContent::Data => 0,
            DataContent::PositionDeletes => 1,
            DataContent::EqualityDeletes => 2,
        }
    }
}

/// One data file referenced by a manifest entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFile {
    pub file_path: String,
    pub file_format: DataFileFormat,
    pub partition_spec_id: i32,
    pub content: DataContent,
    pub record_count: u64,
    pub file_size_bytes: u64,
}

/// One manifest entry — file + lifecycle status + snapshot provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub status: ManifestEntryStatus,
    pub snapshot_id: Option<i64>,
    pub data_file: DataFile,
    pub sequence_number: Option<i64>,
}

impl ManifestEntry {
    pub fn added(snapshot_id: i64, data_file: DataFile) -> Self {
        Self {
            status: ManifestEntryStatus::Added,
            snapshot_id: Some(snapshot_id),
            data_file,
            sequence_number: None,
        }
    }
}

/// Iceberg Manifest — list of entries belonging to one snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub manifest_path: String,
    pub manifest_length: u64,
    pub partition_spec_id: i32,
    pub added_files_count: u32,
    pub existing_files_count: u32,
    pub deleted_files_count: u32,
    pub entries: Vec<ManifestEntry>,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
}

impl Manifest {
    pub fn new(path: impl Into<String>, partition_spec_id: i32) -> Self {
        Self {
            manifest_path: path.into(),
            manifest_length: 0,
            partition_spec_id,
            added_files_count: 0,
            existing_files_count: 0,
            deleted_files_count: 0,
            entries: Vec::new(),
            tenant_id: default_tenant_id(),
        }
    }

    pub fn with_tenant(mut self, t: impl Into<String>) -> Self {
        self.tenant_id = t.into();
        self
    }

    pub fn push(&mut self, entry: ManifestEntry) {
        match entry.status {
            ManifestEntryStatus::Added => self.added_files_count += 1,
            ManifestEntryStatus::Existing => self.existing_files_count += 1,
            ManifestEntryStatus::Deleted => self.deleted_files_count += 1,
        }
        self.entries.push(entry);
    }

    /// Validate counters match `entries`, all entries share `partition_spec_id`,
    /// tenant_id is valid.
    pub fn validate(&self) -> IcebergResult<()> {
        validate_tenant_id(&self.tenant_id)?;
        let mut a = 0u32;
        let mut e = 0u32;
        let mut d = 0u32;
        for entry in &self.entries {
            if entry.data_file.partition_spec_id != self.partition_spec_id {
                return Err(IcebergError::Manifest(format!(
                    "entry partition_spec_id {} != manifest spec_id {}",
                    entry.data_file.partition_spec_id, self.partition_spec_id
                )));
            }
            match entry.status {
                ManifestEntryStatus::Added => a += 1,
                ManifestEntryStatus::Existing => e += 1,
                ManifestEntryStatus::Deleted => d += 1,
            }
        }
        if a != self.added_files_count
            || e != self.existing_files_count
            || d != self.deleted_files_count
        {
            return Err(IcebergError::Manifest(format!(
                "counters mismatch: added {}/{} existing {}/{} deleted {}/{}",
                a, self.added_files_count, e, self.existing_files_count, d, self.deleted_files_count
            )));
        }
        Ok(())
    }

    pub fn total_record_count(&self) -> u64 {
        self.entries
            .iter()
            .filter(|e| e.status != ManifestEntryStatus::Deleted)
            .map(|e| e.data_file.record_count)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_file(id: u32, spec_id: i32) -> DataFile {
        DataFile {
            file_path: format!("/lake/data/file-{:08}.parquet", id),
            file_format: DataFileFormat::Parquet,
            partition_spec_id: spec_id,
            content: DataContent::Data,
            record_count: 1000,
            file_size_bytes: 100_000,
        }
    }

    // ── Codes ─────────────────────────────────────────────────────────────────

    #[test]
    fn entry_status_codes_match_spec() {
        // citation: iceberg spec ManifestEntry.status enum 0/1/2
        assert_eq!(ManifestEntryStatus::Existing.code(), 0);
        assert_eq!(ManifestEntryStatus::Added.code(), 1);
        assert_eq!(ManifestEntryStatus::Deleted.code(), 2);
    }

    #[test]
    fn data_content_codes_match_spec_v2() {
        assert_eq!(DataContent::Data.code(), 0);
        assert_eq!(DataContent::PositionDeletes.code(), 1);
        assert_eq!(DataContent::EqualityDeletes.code(), 2);
    }

    // ── DataFile serde ────────────────────────────────────────────────────────

    #[test]
    fn data_file_serde_round_trip() {
        let f = data_file(1, 0);
        let j = serde_json::to_string(&f).unwrap();
        let back: DataFile = serde_json::from_str(&j).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn data_file_format_uppercase_in_json() {
        let f = data_file(1, 0);
        let j = serde_json::to_string(&f).unwrap();
        assert!(j.contains("\"PARQUET\""));
    }

    #[test]
    fn data_file_format_round_trip_all() {
        for f in [DataFileFormat::Parquet, DataFileFormat::Avro, DataFileFormat::Orc] {
            let j = serde_json::to_string(&f).unwrap();
            let back: DataFileFormat = serde_json::from_str(&j).unwrap();
            assert_eq!(back, f);
        }
    }

    // ── ManifestEntry constructors ────────────────────────────────────────────

    #[test]
    fn manifest_entry_added_constructor() {
        let f = data_file(1, 0);
        let e = ManifestEntry::added(42, f.clone());
        assert_eq!(e.status, ManifestEntryStatus::Added);
        assert_eq!(e.snapshot_id, Some(42));
        assert_eq!(e.data_file, f);
    }

    // ── Manifest constructors + push ──────────────────────────────────────────

    #[test]
    fn manifest_default_tenant() {
        let m = Manifest::new("/lake/m1.avro", 0);
        assert_eq!(m.tenant_id, "default");
    }

    #[test]
    fn manifest_with_tenant() {
        let m = Manifest::new("/lake/m1.avro", 0).with_tenant("acme");
        assert_eq!(m.tenant_id, "acme");
    }

    #[test]
    fn manifest_push_added_increments_counter() {
        let mut m = Manifest::new("/lake/m.avro", 0);
        m.push(ManifestEntry::added(1, data_file(1, 0)));
        m.push(ManifestEntry::added(1, data_file(2, 0)));
        assert_eq!(m.added_files_count, 2);
        assert_eq!(m.existing_files_count, 0);
        assert_eq!(m.entries.len(), 2);
    }

    #[test]
    fn manifest_push_mixed_increments_correct_counters() {
        let mut m = Manifest::new("/lake/m.avro", 0);
        m.push(ManifestEntry::added(1, data_file(1, 0)));
        m.push(ManifestEntry {
            status: ManifestEntryStatus::Existing,
            snapshot_id: Some(1),
            data_file: data_file(2, 0),
            sequence_number: None,
        });
        m.push(ManifestEntry {
            status: ManifestEntryStatus::Deleted,
            snapshot_id: Some(1),
            data_file: data_file(3, 0),
            sequence_number: None,
        });
        assert_eq!(m.added_files_count, 1);
        assert_eq!(m.existing_files_count, 1);
        assert_eq!(m.deleted_files_count, 1);
    }

    // ── Manifest validate ─────────────────────────────────────────────────────

    #[test]
    fn manifest_validate_default_ok() {
        let m = Manifest::new("/lake/m.avro", 0);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn manifest_validate_with_entries_ok() {
        let mut m = Manifest::new("/lake/m.avro", 0);
        m.push(ManifestEntry::added(1, data_file(1, 0)));
        m.push(ManifestEntry::added(1, data_file(2, 0)));
        assert!(m.validate().is_ok());
    }

    #[test]
    fn manifest_validate_partition_spec_mismatch_err() {
        let mut m = Manifest::new("/lake/m.avro", 0);
        m.push(ManifestEntry::added(1, data_file(1, 7))); // wrong spec_id
        let e = m.validate().unwrap_err().to_string();
        assert!(e.contains("partition_spec_id"));
    }

    #[test]
    fn manifest_validate_counter_mismatch_err() {
        let mut m = Manifest::new("/lake/m.avro", 0);
        m.added_files_count = 5; // lie about counters
        let e = m.validate().unwrap_err().to_string();
        assert!(e.contains("counters mismatch"));
    }

    #[test]
    fn manifest_validate_invalid_tenant_err() {
        let mut m = Manifest::new("/lake/m.avro", 0);
        m.tenant_id = "BAD".into();
        assert!(m.validate().is_err());
    }

    // ── total_record_count ────────────────────────────────────────────────────

    #[test]
    fn total_record_count_excludes_deleted() {
        let mut m = Manifest::new("/lake/m.avro", 0);
        m.push(ManifestEntry::added(1, data_file(1, 0))); // 1000
        m.push(ManifestEntry::added(1, data_file(2, 0))); // 1000
        m.push(ManifestEntry {
            status: ManifestEntryStatus::Deleted,
            snapshot_id: Some(1),
            data_file: data_file(3, 0),
            sequence_number: None,
        });
        assert_eq!(m.total_record_count(), 2000);
    }

    #[test]
    fn total_record_count_zero_when_empty() {
        let m = Manifest::new("/lake/m.avro", 0);
        assert_eq!(m.total_record_count(), 0);
    }

    // ── serde round-trip ──────────────────────────────────────────────────────

    #[test]
    fn manifest_serde_round_trip() {
        let mut m = Manifest::new("/lake/m.avro", 0).with_tenant("acme");
        m.push(ManifestEntry::added(1, data_file(1, 0)));
        let j = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&j).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn manifest_deserialize_omitted_tenant_defaults() {
        let j = r#"{"manifest_path":"/lake/x","manifest_length":0,"partition_spec_id":0,"added_files_count":0,"existing_files_count":0,"deleted_files_count":0,"entries":[]}"#;
        let m: Manifest = serde_json::from_str(j).unwrap();
        assert_eq!(m.tenant_id, "default");
    }
}
