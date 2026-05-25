// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg Manifest-list file.
//!
//! Upstream:
//! * `crates/iceberg/src/spec/manifest_list.rs`
//! Spec: <https://iceberg.apache.org/spec/#manifest-lists>

use crate::manifest::DataFileContent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ManifestListContent {
    Data = 0,
    Deletes = 1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestFile {
    pub manifest_path: String,
    pub manifest_length: i64,
    pub partition_spec_id: i32,
    pub content: ManifestListContent,
    pub sequence_number: i64,
    pub min_sequence_number: i64,
    pub added_snapshot_id: i64,
    /// Counts from the manifest, for fast pruning.
    pub added_data_files_count: i32,
    pub existing_data_files_count: i32,
    pub deleted_data_files_count: i32,
    pub added_rows_count: i64,
    pub existing_rows_count: i64,
    pub deleted_rows_count: i64,
}

impl ManifestFile {
    pub fn live_rows(&self) -> i64 {
        self.added_rows_count + self.existing_rows_count - self.deleted_rows_count
    }

    pub fn live_files(&self) -> i32 {
        self.added_data_files_count + self.existing_data_files_count - self.deleted_data_files_count
    }

    pub fn is_data(&self) -> bool {
        matches!(self.content, ManifestListContent::Data)
    }

    pub fn matches_content(&self, want: DataFileContent) -> bool {
        match (self.content, want) {
            (ManifestListContent::Data, DataFileContent::Data) => true,
            (ManifestListContent::Deletes, DataFileContent::PositionDeletes) => true,
            (ManifestListContent::Deletes, DataFileContent::EqualityDeletes) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ManifestList {
    pub entries: Vec<ManifestFile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mf(content: ManifestListContent, added: i64, deleted: i64) -> ManifestFile {
        ManifestFile {
            manifest_path: "s3://x/m.avro".into(),
            manifest_length: 1024,
            partition_spec_id: 0,
            content,
            sequence_number: 1,
            min_sequence_number: 1,
            added_snapshot_id: 1,
            added_data_files_count: 1,
            existing_data_files_count: 0,
            deleted_data_files_count: 0,
            added_rows_count: added,
            existing_rows_count: 0,
            deleted_rows_count: deleted,
        }
    }

    #[test]
    fn live_rows_subtracts_deleted() {
        let f = mf(ManifestListContent::Data, 100, 30);
        assert_eq!(f.live_rows(), 70);
    }

    #[test]
    fn is_data_distinguishes_content() {
        assert!(mf(ManifestListContent::Data, 1, 0).is_data());
        assert!(!mf(ManifestListContent::Deletes, 1, 0).is_data());
    }

    #[test]
    fn matches_content_routes_correctly() {
        let d = mf(ManifestListContent::Data, 1, 0);
        let dl = mf(ManifestListContent::Deletes, 1, 0);
        assert!(d.matches_content(DataFileContent::Data));
        assert!(!d.matches_content(DataFileContent::EqualityDeletes));
        assert!(dl.matches_content(DataFileContent::PositionDeletes));
        assert!(dl.matches_content(DataFileContent::EqualityDeletes));
    }
}
