//! Iceberg Snapshot — atomic table version pointing at a manifest list.
//!
//! Mirrors apache/iceberg-rust crates/iceberg/src/spec/snapshot.rs and
//! the spec at https://iceberg.apache.org/spec/#snapshots.

use crate::error::{IcebergError, IcebergResult};
use crate::tenant::{default_tenant_id, validate_tenant_id};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Operation that produced this snapshot. Matches iceberg `summary.operation`
/// values in spec — append / overwrite / replace / delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotOperation {
    Append,
    Overwrite,
    Replace,
    Delete,
}

impl SnapshotOperation {
    pub const fn spec_name(self) -> &'static str {
        match self {
            SnapshotOperation::Append => "append",
            SnapshotOperation::Overwrite => "overwrite",
            SnapshotOperation::Replace => "replace",
            SnapshotOperation::Delete => "delete",
        }
    }
}

/// Iceberg `snapshot.summary` — operation + arbitrary key/value stats
/// (added-records, total-records, …). Matches spec/snapshot.rs `Summary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSummary {
    pub operation: SnapshotOperation,
    #[serde(default)]
    pub additional_properties: HashMap<String, String>,
}

impl SnapshotSummary {
    pub fn new(op: SnapshotOperation) -> Self {
        Self {
            operation: op,
            additional_properties: HashMap::new(),
        }
    }

    pub fn with(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.additional_properties.insert(k.into(), v.into());
        self
    }

    pub fn added_records(&self) -> Option<u64> {
        self.additional_properties
            .get("added-records")
            .and_then(|s| s.parse().ok())
    }

    pub fn total_records(&self) -> Option<u64> {
        self.additional_properties
            .get("total-records")
            .and_then(|s| s.parse().ok())
    }
}

/// One Iceberg Snapshot entry in a TableMetadata log.
///
/// `parent_snapshot_id` is None for the very first snapshot. `manifest_list`
/// is the path to the avro file listing all manifests of this snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub snapshot_id: i64,
    pub parent_snapshot_id: Option<i64>,
    pub sequence_number: i64,
    pub timestamp_ms: i64,
    pub manifest_list: String,
    pub summary: SnapshotSummary,
    pub schema_id: Option<i32>,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
}

impl Snapshot {
    pub fn new(
        snapshot_id: i64,
        sequence_number: i64,
        timestamp_ms: i64,
        manifest_list: impl Into<String>,
        operation: SnapshotOperation,
    ) -> Self {
        Self {
            snapshot_id,
            parent_snapshot_id: None,
            sequence_number,
            timestamp_ms,
            manifest_list: manifest_list.into(),
            summary: SnapshotSummary::new(operation),
            schema_id: None,
            tenant_id: default_tenant_id(),
        }
    }

    pub fn with_parent(mut self, parent: i64) -> Self {
        self.parent_snapshot_id = Some(parent);
        self
    }

    pub fn with_tenant(mut self, t: impl Into<String>) -> Self {
        self.tenant_id = t.into();
        self
    }

    pub fn with_schema(mut self, schema_id: i32) -> Self {
        self.schema_id = Some(schema_id);
        self
    }

    pub fn validate(&self) -> IcebergResult<()> {
        validate_tenant_id(&self.tenant_id)?;
        if self.snapshot_id == 0 {
            return Err(IcebergError::Snapshot("snapshot_id must be non-zero".into()));
        }
        if self.sequence_number < 0 {
            return Err(IcebergError::Snapshot("sequence_number must be ≥ 0".into()));
        }
        if self.timestamp_ms < 0 {
            return Err(IcebergError::Snapshot("timestamp_ms must be ≥ 0".into()));
        }
        if self.manifest_list.is_empty() {
            return Err(IcebergError::Snapshot("manifest_list path must not be empty".into()));
        }
        if let Some(p) = self.parent_snapshot_id {
            if p == self.snapshot_id {
                return Err(IcebergError::Snapshot(
                    "parent_snapshot_id must differ from snapshot_id".into(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snapshot {
        Snapshot::new(
            42,
            1,
            1_700_000_000_000,
            "/lake/snap-42-manifest-list.avro",
            SnapshotOperation::Append,
        )
    }

    // ── SnapshotOperation ──────────────────────────────────────────────────────

    #[test]
    fn op_spec_names() {
        // citation: iceberg spec snapshot.summary.operation values
        assert_eq!(SnapshotOperation::Append.spec_name(), "append");
        assert_eq!(SnapshotOperation::Overwrite.spec_name(), "overwrite");
        assert_eq!(SnapshotOperation::Replace.spec_name(), "replace");
        assert_eq!(SnapshotOperation::Delete.spec_name(), "delete");
    }

    #[test]
    fn op_serde_lowercase() {
        let j = serde_json::to_string(&SnapshotOperation::Append).unwrap();
        assert_eq!(j, "\"append\"");
    }

    #[test]
    fn op_round_trip_all() {
        for o in [
            SnapshotOperation::Append,
            SnapshotOperation::Overwrite,
            SnapshotOperation::Replace,
            SnapshotOperation::Delete,
        ] {
            let j = serde_json::to_string(&o).unwrap();
            let back: SnapshotOperation = serde_json::from_str(&j).unwrap();
            assert_eq!(back, o);
        }
    }

    // ── SnapshotSummary ────────────────────────────────────────────────────────

    #[test]
    fn summary_with_appends_property() {
        let s = SnapshotSummary::new(SnapshotOperation::Append).with("added-records", "1000");
        assert_eq!(s.additional_properties.get("added-records").unwrap(), "1000");
    }

    #[test]
    fn summary_added_records_typed() {
        let s = SnapshotSummary::new(SnapshotOperation::Append).with("added-records", "1500");
        assert_eq!(s.added_records(), Some(1500));
    }

    #[test]
    fn summary_total_records_typed() {
        let s = SnapshotSummary::new(SnapshotOperation::Append).with("total-records", "10000");
        assert_eq!(s.total_records(), Some(10000));
    }

    #[test]
    fn summary_added_records_missing_returns_none() {
        let s = SnapshotSummary::new(SnapshotOperation::Append);
        assert_eq!(s.added_records(), None);
    }

    #[test]
    fn summary_added_records_non_numeric_returns_none() {
        let s = SnapshotSummary::new(SnapshotOperation::Append).with("added-records", "abc");
        assert_eq!(s.added_records(), None);
    }

    // ── Snapshot constructors ─────────────────────────────────────────────────

    #[test]
    fn snapshot_default_has_no_parent() {
        let s = snap();
        assert!(s.parent_snapshot_id.is_none());
    }

    #[test]
    fn snapshot_with_parent() {
        let s = snap().with_parent(100);
        assert_eq!(s.parent_snapshot_id, Some(100));
    }

    #[test]
    fn snapshot_with_tenant() {
        let s = snap().with_tenant("acme");
        assert_eq!(s.tenant_id, "acme");
    }

    #[test]
    fn snapshot_with_schema() {
        let s = snap().with_schema(7);
        assert_eq!(s.schema_id, Some(7));
    }

    #[test]
    fn snapshot_default_tenant_is_default() {
        assert_eq!(snap().tenant_id, "default");
    }

    // ── Snapshot validate ─────────────────────────────────────────────────────

    #[test]
    fn snapshot_validate_default_ok() {
        assert!(snap().validate().is_ok());
    }

    #[test]
    fn snapshot_validate_zero_id_err() {
        let mut s = snap();
        s.snapshot_id = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn snapshot_validate_negative_seq_err() {
        let mut s = snap();
        s.sequence_number = -1;
        assert!(s.validate().is_err());
    }

    #[test]
    fn snapshot_validate_negative_ts_err() {
        let mut s = snap();
        s.timestamp_ms = -1;
        assert!(s.validate().is_err());
    }

    #[test]
    fn snapshot_validate_empty_manifest_list_err() {
        let mut s = snap();
        s.manifest_list = "".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn snapshot_validate_self_parent_err() {
        let s = snap().with_parent(42); // same as snapshot_id
        let e = s.validate().unwrap_err().to_string();
        assert!(e.contains("parent_snapshot_id"));
    }

    #[test]
    fn snapshot_validate_invalid_tenant_err() {
        let s = snap().with_tenant("BAD");
        assert!(s.validate().is_err());
    }

    // ── Snapshot serde ────────────────────────────────────────────────────────

    #[test]
    fn snapshot_serde_round_trip() {
        let s = snap()
            .with_parent(1)
            .with_tenant("acme")
            .with_schema(0);
        let j = serde_json::to_string(&s).unwrap();
        let back: Snapshot = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn snapshot_deserialize_omitted_tenant_defaults() {
        let j = r#"{"snapshot_id":1,"parent_snapshot_id":null,"sequence_number":0,"timestamp_ms":0,"manifest_list":"x","summary":{"operation":"append","additional_properties":{}},"schema_id":null}"#;
        let s: Snapshot = serde_json::from_str(j).unwrap();
        assert_eq!(s.tenant_id, "default");
    }
}
