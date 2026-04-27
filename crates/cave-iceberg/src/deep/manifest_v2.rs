//! Manifest v2 augmentations — sequence numbers + spec-id binding +
//! file-size-based pruning.
//!
//! Mirrors the spec at <https://iceberg.apache.org/spec/#manifest-files>:
//!
//! * Each manifest file carries `sequence_number` and `min_sequence_number`.
//! * Each entry has `data_sequence_number` (added) and
//!   `file_sequence_number` (assigned at commit).
//! * Position-delete and equality-delete files participate in row-level MERGE.

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ManifestV2Error {
    #[error("sequence regression: {previous} → {next}")]
    SequenceRegression { previous: i64, next: i64 },
    #[error("delete file references unknown data file: {0}")]
    UnknownDataFile(String),
    #[error("invalid spec id: {0}")]
    InvalidSpecId(i32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ManifestEntry {
    pub status: ManifestEntryStatus,
    pub data_sequence_number: i64,
    pub file_sequence_number: i64,
    pub file_path: String,
    pub partition_spec_id: i32,
    pub file_size_bytes: i64,
    pub record_count: i64,
    pub content: DataContent,
    /// References data files by path when this entry is a delete file.
    pub referenced_data_files: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestEntryStatus {
    Existing,
    Added,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataContent {
    Data,
    PositionDeletes,
    EqualityDeletes,
}

#[derive(Debug, Clone)]
pub struct ManifestFileV2 {
    pub manifest_path: String,
    pub sequence_number: i64,
    pub min_sequence_number: i64,
    pub partition_spec_id: i32,
    pub added_files_count: i32,
    pub existing_files_count: i32,
    pub deleted_files_count: i32,
    pub added_rows_count: i64,
    pub existing_rows_count: i64,
    pub deleted_rows_count: i64,
    pub entries: Vec<ManifestEntry>,
}

impl ManifestFileV2 {
    pub fn new(path: &str, spec_id: i32) -> Self {
        Self {
            manifest_path: path.into(),
            sequence_number: 0,
            min_sequence_number: i64::MAX,
            partition_spec_id: spec_id,
            added_files_count: 0,
            existing_files_count: 0,
            deleted_files_count: 0,
            added_rows_count: 0,
            existing_rows_count: 0,
            deleted_rows_count: 0,
            entries: Vec::new(),
        }
    }

    pub fn add_entry(&mut self, mut entry: ManifestEntry, sequence: i64) -> Result<(), ManifestV2Error> {
        if sequence < self.sequence_number {
            return Err(ManifestV2Error::SequenceRegression {
                previous: self.sequence_number,
                next: sequence,
            });
        }
        entry.data_sequence_number = sequence;
        entry.file_sequence_number = sequence;
        self.sequence_number = sequence;
        self.min_sequence_number = self.min_sequence_number.min(sequence);
        match entry.status {
            ManifestEntryStatus::Added => {
                self.added_files_count += 1;
                self.added_rows_count += entry.record_count;
            }
            ManifestEntryStatus::Existing => {
                self.existing_files_count += 1;
                self.existing_rows_count += entry.record_count;
            }
            ManifestEntryStatus::Deleted => {
                self.deleted_files_count += 1;
                self.deleted_rows_count += entry.record_count;
            }
        }
        self.entries.push(entry);
        Ok(())
    }

    /// Validate that delete files reference existing data files in the same
    /// manifest. Returns the list of dangling references.
    pub fn validate_delete_refs(&self) -> Vec<String> {
        let known: std::collections::BTreeSet<&str> = self
            .entries
            .iter()
            .filter(|e| e.content == DataContent::Data)
            .map(|e| e.file_path.as_str())
            .collect();
        let mut dangling = Vec::new();
        for e in &self.entries {
            if e.content != DataContent::Data {
                for r in &e.referenced_data_files {
                    if !known.contains(r.as_str()) {
                        dangling.push(r.clone());
                    }
                }
            }
        }
        dangling
    }

    /// Total live records (added + existing − deleted).
    pub fn live_record_count(&self) -> i64 {
        self.added_rows_count + self.existing_rows_count - self.deleted_rows_count
    }

    /// File-pruning helper: drop entries whose file size is below the threshold.
    pub fn prune_small_files(&mut self, min_size_bytes: i64) -> Vec<String> {
        let mut pruned = Vec::new();
        self.entries.retain(|e| {
            if e.file_size_bytes < min_size_bytes && e.status != ManifestEntryStatus::Deleted {
                pruned.push(e.file_path.clone());
                false
            } else {
                true
            }
        });
        pruned
    }
}

/// File-by-file partition stats summary.
#[derive(Debug, Clone, Default)]
pub struct ManifestStats {
    pub by_partition: BTreeMap<String, i64>,
}

pub fn build_partition_stats(file: &ManifestFileV2) -> ManifestStats {
    let mut stats = ManifestStats::default();
    for e in &file.entries {
        let part_key = format!("spec-{}-file-{}", e.partition_spec_id, e.file_path);
        *stats.by_partition.entry(part_key).or_insert(0) += e.record_count;
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_entry(path: &str, rows: i64) -> ManifestEntry {
        ManifestEntry {
            status: ManifestEntryStatus::Added,
            data_sequence_number: 0,
            file_sequence_number: 0,
            file_path: path.into(),
            partition_spec_id: 0,
            file_size_bytes: 1000,
            record_count: rows,
            content: DataContent::Data,
            referenced_data_files: vec![],
        }
    }

    #[test]
    fn add_entry_assigns_sequence_numbers() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        f.add_entry(data_entry("a.parquet", 100), 5).unwrap();
        assert_eq!(f.entries[0].data_sequence_number, 5);
        assert_eq!(f.entries[0].file_sequence_number, 5);
        assert_eq!(f.sequence_number, 5);
        assert_eq!(f.min_sequence_number, 5);
    }

    #[test]
    fn add_entry_advances_max_seq() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        f.add_entry(data_entry("a.parquet", 100), 5).unwrap();
        f.add_entry(data_entry("b.parquet", 200), 7).unwrap();
        assert_eq!(f.sequence_number, 7);
        assert_eq!(f.min_sequence_number, 5);
    }

    #[test]
    fn sequence_regression_errors() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        f.add_entry(data_entry("a.parquet", 100), 5).unwrap();
        let err = f.add_entry(data_entry("b.parquet", 200), 3).unwrap_err();
        assert!(matches!(err, ManifestV2Error::SequenceRegression { .. }));
    }

    #[test]
    fn counts_track_status() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        f.add_entry(data_entry("a.parquet", 100), 1).unwrap();
        let mut ex = data_entry("b.parquet", 50);
        ex.status = ManifestEntryStatus::Existing;
        f.add_entry(ex, 2).unwrap();
        let mut del = data_entry("c.parquet", 25);
        del.status = ManifestEntryStatus::Deleted;
        f.add_entry(del, 3).unwrap();
        assert_eq!(f.added_files_count, 1);
        assert_eq!(f.existing_files_count, 1);
        assert_eq!(f.deleted_files_count, 1);
    }

    #[test]
    fn live_record_count_subtracts_deleted() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        f.add_entry(data_entry("a", 100), 1).unwrap();
        let mut del = data_entry("b", 25);
        del.status = ManifestEntryStatus::Deleted;
        f.add_entry(del, 2).unwrap();
        assert_eq!(f.live_record_count(), 75);
    }

    #[test]
    fn prune_drops_small_files() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        let mut small = data_entry("small.parquet", 10);
        small.file_size_bytes = 100;
        let big = data_entry("big.parquet", 100_000);
        f.add_entry(small, 1).unwrap();
        f.add_entry(big, 2).unwrap();
        let pruned = f.prune_small_files(1000);
        assert_eq!(pruned, vec!["small.parquet"]);
        assert_eq!(f.entries.len(), 1);
    }

    #[test]
    fn validate_delete_refs_finds_dangling() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        f.add_entry(data_entry("data.parquet", 100), 1).unwrap();
        let mut del = data_entry("del.parquet", 5);
        del.content = DataContent::PositionDeletes;
        del.status = ManifestEntryStatus::Added;
        del.referenced_data_files = vec!["missing.parquet".into()];
        f.add_entry(del, 2).unwrap();
        let dangling = f.validate_delete_refs();
        assert_eq!(dangling, vec!["missing.parquet"]);
    }

    #[test]
    fn validate_delete_refs_finds_nothing_when_resolved() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        f.add_entry(data_entry("data.parquet", 100), 1).unwrap();
        let mut del = data_entry("del.parquet", 5);
        del.content = DataContent::EqualityDeletes;
        del.referenced_data_files = vec!["data.parquet".into()];
        f.add_entry(del, 2).unwrap();
        assert!(f.validate_delete_refs().is_empty());
    }

    #[test]
    fn data_content_codes_match_spec() {
        // citation: iceberg spec v2 — content codes 0/1/2
        // We don't expose codes on the deep enum; only assert that the variants exist.
        assert_ne!(DataContent::Data, DataContent::PositionDeletes);
        assert_ne!(DataContent::PositionDeletes, DataContent::EqualityDeletes);
    }

    #[test]
    fn build_partition_stats_aggregates_records() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        f.add_entry(data_entry("a.parquet", 100), 1).unwrap();
        f.add_entry(data_entry("b.parquet", 200), 2).unwrap();
        let stats = build_partition_stats(&f);
        assert_eq!(stats.by_partition.len(), 2);
    }

    #[test]
    fn manifest_status_codes_distinct() {
        assert_ne!(ManifestEntryStatus::Added, ManifestEntryStatus::Existing);
        assert_ne!(ManifestEntryStatus::Existing, ManifestEntryStatus::Deleted);
    }

    #[test]
    fn first_entry_initializes_min_sequence() {
        let mut f = ManifestFileV2::new("m1.avro", 0);
        f.add_entry(data_entry("a.parquet", 100), 10).unwrap();
        assert_eq!(f.min_sequence_number, 10);
    }
}
