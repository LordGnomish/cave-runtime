// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg Snapshot + SnapshotRef.
//!
//! Upstream:
//! * `crates/iceberg/src/spec/snapshot.rs`
//!
//! A Snapshot points at the manifest-list file containing the
//! manifests for that version of the table. Snapshots form a DAG
//! (`parent_snapshot_id` chain) so that time-travel reads can walk
//! backwards. The MVP supports both `append` and `overwrite`
//! summary kinds — that's enough to navigate snapshots; the
//! transaction commit path is deferred.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    #[serde(rename = "snapshot-id")]
    pub snapshot_id: i64,
    #[serde(rename = "parent-snapshot-id", skip_serializing_if = "Option::is_none")]
    pub parent_snapshot_id: Option<i64>,
    #[serde(rename = "sequence-number", default)]
    pub sequence_number: i64,
    #[serde(rename = "timestamp-ms")]
    pub timestamp_ms: i64,
    #[serde(rename = "manifest-list")]
    pub manifest_list: String,
    /// Operation summary — `operation` field has values like "append",
    /// "overwrite", "replace", "delete". Other keys are free-form.
    #[serde(default)]
    pub summary: HashMap<String, String>,
    #[serde(rename = "schema-id", skip_serializing_if = "Option::is_none")]
    pub schema_id: Option<i32>,
}

impl Snapshot {
    pub fn operation(&self) -> Option<&str> {
        self.summary.get("operation").map(String::as_str)
    }
}

/// A named reference (branch / tag) → snapshot-id binding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotRef {
    #[serde(rename = "snapshot-id")]
    pub snapshot_id: i64,
    #[serde(rename = "type")]
    pub ref_type: RefType,
    /// Optional minimum snapshot-to-keep age, expressed as ms (branch only).
    #[serde(rename = "min-snapshots-to-keep", skip_serializing_if = "Option::is_none")]
    pub min_snapshots_to_keep: Option<i32>,
    #[serde(rename = "max-snapshot-age-ms", skip_serializing_if = "Option::is_none")]
    pub max_snapshot_age_ms: Option<i64>,
    /// `max-ref-age-ms` controls automatic GC of the named reference.
    #[serde(rename = "max-ref-age-ms", skip_serializing_if = "Option::is_none")]
    pub max_ref_age_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RefType {
    Branch,
    Tag,
}

impl SnapshotRef {
    pub fn branch(snapshot_id: i64) -> Self {
        Self {
            snapshot_id,
            ref_type: RefType::Branch,
            min_snapshots_to_keep: None,
            max_snapshot_age_ms: None,
            max_ref_age_ms: None,
        }
    }

    pub fn tag(snapshot_id: i64) -> Self {
        Self {
            snapshot_id,
            ref_type: RefType::Tag,
            min_snapshots_to_keep: None,
            max_snapshot_age_ms: None,
            max_ref_age_ms: None,
        }
    }
}

/// Walk back from `head` along `parent_snapshot_id` until either
/// nil-parent or a max of `limit` rows is reached. Used for time-travel
/// reads and ancestor checks.
pub fn ancestors_of(snapshots: &[Snapshot], head: i64, limit: usize) -> Result<Vec<i64>> {
    let by_id: HashMap<i64, &Snapshot> = snapshots.iter().map(|s| (s.snapshot_id, s)).collect();
    let mut out = Vec::new();
    let mut cur = Some(head);
    while let Some(id) = cur {
        if out.len() >= limit {
            break;
        }
        out.push(id);
        let s = by_id
            .get(&id)
            .copied()
            .ok_or(Error::SnapshotNotFound(id))?;
        cur = s.parent_snapshot_id;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(id: i64, parent: Option<i64>) -> Snapshot {
        Snapshot {
            snapshot_id: id,
            parent_snapshot_id: parent,
            sequence_number: id,
            timestamp_ms: 0,
            manifest_list: format!("s3://x/manifest-{}.avro", id),
            summary: HashMap::from_iter([("operation".to_string(), "append".to_string())]),
            schema_id: Some(0),
        }
    }

    #[test]
    fn operation_extraction() {
        let s = snap(1, None);
        assert_eq!(s.operation(), Some("append"));
    }

    #[test]
    fn ancestors_walk_parent_chain() {
        let s1 = snap(1, None);
        let s2 = snap(2, Some(1));
        let s3 = snap(3, Some(2));
        let chain = ancestors_of(&[s1.clone(), s2.clone(), s3.clone()], 3, 10).unwrap();
        assert_eq!(chain, vec![3, 2, 1]);
    }

    #[test]
    fn ancestors_respect_limit() {
        let s1 = snap(1, None);
        let s2 = snap(2, Some(1));
        let s3 = snap(3, Some(2));
        let chain = ancestors_of(&[s1, s2, s3], 3, 2).unwrap();
        assert_eq!(chain, vec![3, 2]);
    }

    #[test]
    fn ancestors_errors_on_missing_parent() {
        let s2 = snap(2, Some(99));
        let r = ancestors_of(&[s2], 2, 10);
        assert!(matches!(r, Err(Error::SnapshotNotFound(99))));
    }

    #[test]
    fn snapshot_ref_branch_and_tag() {
        let b = SnapshotRef::branch(7);
        let t = SnapshotRef::tag(7);
        assert!(matches!(b.ref_type, RefType::Branch));
        assert!(matches!(t.ref_type, RefType::Tag));
    }

    #[test]
    fn snapshot_serializes_kebab_keys() {
        let s = snap(1, Some(0));
        let j = serde_json::to_value(&s).unwrap();
        assert!(j.get("snapshot-id").is_some());
        assert!(j.get("manifest-list").is_some());
    }
}
