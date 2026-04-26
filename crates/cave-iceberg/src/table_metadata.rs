//! Iceberg TableMetadata — top-level table descriptor (`metadata.json`).
//!
//! Mirrors apache/iceberg-rust crates/iceberg/src/spec/table_metadata.rs and
//! the spec at https://iceberg.apache.org/spec/#table-metadata.

use crate::error::{IcebergError, IcebergResult};
use crate::partition::PartitionSpec;
use crate::schema::Schema;
use crate::snapshot::Snapshot;
use crate::tenant::{default_tenant_id, validate_tenant_id};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Iceberg spec format version. Format-v1 is legacy; v2 is the default for
/// new tables (apache/iceberg spec/format-version).
pub const FORMAT_VERSION_V2: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableMetadata {
    pub format_version: i32,
    pub table_uuid: Uuid,
    pub location: String,
    pub last_sequence_number: i64,
    pub last_updated_ms: i64,
    pub last_column_id: i32,
    pub schemas: Vec<Schema>,
    pub current_schema_id: i32,
    pub partition_specs: Vec<PartitionSpec>,
    pub default_spec_id: i32,
    pub snapshots: Vec<Snapshot>,
    pub current_snapshot_id: Option<i64>,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
}

impl TableMetadata {
    pub fn new(location: impl Into<String>, schema: Schema, spec: PartitionSpec) -> Self {
        Self {
            format_version: FORMAT_VERSION_V2,
            table_uuid: Uuid::new_v4(),
            location: location.into(),
            last_sequence_number: 0,
            last_updated_ms: 0,
            last_column_id: schema.fields.iter().map(|f| f.id).max().unwrap_or(0),
            current_schema_id: schema.schema_id,
            schemas: vec![schema],
            default_spec_id: spec.spec_id,
            partition_specs: vec![spec],
            snapshots: Vec::new(),
            current_snapshot_id: None,
            tenant_id: default_tenant_id(),
        }
    }

    pub fn with_tenant(mut self, t: impl Into<String>) -> Self {
        self.tenant_id = t.into();
        self
    }

    pub fn current_schema(&self) -> Option<&Schema> {
        self.schemas
            .iter()
            .find(|s| s.schema_id == self.current_schema_id)
    }

    pub fn default_spec(&self) -> Option<&PartitionSpec> {
        self.partition_specs
            .iter()
            .find(|s| s.spec_id == self.default_spec_id)
    }

    pub fn current_snapshot(&self) -> Option<&Snapshot> {
        self.current_snapshot_id
            .and_then(|id| self.snapshots.iter().find(|s| s.snapshot_id == id))
    }

    pub fn add_snapshot(&mut self, snapshot: Snapshot) -> IcebergResult<()> {
        snapshot.validate()?;
        if snapshot.tenant_id != self.tenant_id {
            return Err(IcebergError::TableMetadata(format!(
                "snapshot tenant '{}' does not match table tenant '{}'",
                snapshot.tenant_id, self.tenant_id
            )));
        }
        if snapshot.sequence_number <= self.last_sequence_number && self.last_sequence_number > 0 {
            return Err(IcebergError::TableMetadata(format!(
                "snapshot sequence_number {} must be > last_sequence_number {}",
                snapshot.sequence_number, self.last_sequence_number
            )));
        }
        self.last_sequence_number = snapshot.sequence_number;
        self.last_updated_ms = snapshot.timestamp_ms;
        self.current_snapshot_id = Some(snapshot.snapshot_id);
        self.snapshots.push(snapshot);
        Ok(())
    }

    /// Validate the metadata document:
    /// - format_version supported
    /// - tenant_id valid + matches every schema/snapshot
    /// - current_schema_id and default_spec_id resolve
    /// - all schemas/specs/snapshots themselves valid
    pub fn validate(&self) -> IcebergResult<()> {
        if self.format_version != FORMAT_VERSION_V2 && self.format_version != 1 {
            return Err(IcebergError::TableMetadata(format!(
                "unsupported format_version {}",
                self.format_version
            )));
        }
        validate_tenant_id(&self.tenant_id)?;
        if self.location.is_empty() {
            return Err(IcebergError::TableMetadata("location must not be empty".into()));
        }
        for s in &self.schemas {
            s.validate()?;
            if s.tenant_id != self.tenant_id {
                return Err(IcebergError::TableMetadata(format!(
                    "schema {} tenant '{}' != table tenant '{}'",
                    s.schema_id, s.tenant_id, self.tenant_id
                )));
            }
        }
        if self.current_schema().is_none() {
            return Err(IcebergError::TableMetadata(format!(
                "current_schema_id {} not found",
                self.current_schema_id
            )));
        }
        for spec in &self.partition_specs {
            let schema = self.current_schema().unwrap();
            spec.validate(schema)?;
        }
        if self.default_spec().is_none() {
            return Err(IcebergError::TableMetadata(format!(
                "default_spec_id {} not found",
                self.default_spec_id
            )));
        }
        for snap in &self.snapshots {
            snap.validate()?;
            if snap.tenant_id != self.tenant_id {
                return Err(IcebergError::TableMetadata(format!(
                    "snapshot {} tenant '{}' != table tenant '{}'",
                    snap.snapshot_id, snap.tenant_id, self.tenant_id
                )));
            }
        }
        if let Some(cur_id) = self.current_snapshot_id {
            if !self.snapshots.iter().any(|s| s.snapshot_id == cur_id) {
                return Err(IcebergError::TableMetadata(format!(
                    "current_snapshot_id {} not found in snapshots",
                    cur_id
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::partition::PartitionSpec;
    use crate::schema::{Field, PrimitiveType, Schema};
    use crate::snapshot::{Snapshot, SnapshotOperation};

    fn schema() -> Schema {
        Schema::new(
            0,
            vec![
                Field::required(1, "id", PrimitiveType::Long),
                Field::required(2, "name", PrimitiveType::String),
            ],
        )
    }

    fn meta() -> TableMetadata {
        TableMetadata::new("/lake/users", schema(), PartitionSpec::unpartitioned(0))
    }

    // ── format version ─────────────────────────────────────────────────────────

    #[test]
    fn format_version_v2_default() {
        // citation: iceberg spec — v2 is default for new tables
        assert_eq!(FORMAT_VERSION_V2, 2);
        assert_eq!(meta().format_version, 2);
    }

    // ── constructor ────────────────────────────────────────────────────────────

    #[test]
    fn new_seeds_last_column_id_from_max_field_id() {
        let m = meta();
        assert_eq!(m.last_column_id, 2);
    }

    #[test]
    fn new_default_tenant_is_default() {
        assert_eq!(meta().tenant_id, "default");
    }

    #[test]
    fn new_seeds_current_schema_id_from_passed_schema() {
        let m = meta();
        assert_eq!(m.current_schema_id, 0);
    }

    #[test]
    fn new_starts_with_no_snapshots() {
        let m = meta();
        assert!(m.snapshots.is_empty());
        assert!(m.current_snapshot_id.is_none());
    }

    // ── lookups ────────────────────────────────────────────────────────────────

    #[test]
    fn current_schema_returns_match() {
        let m = meta();
        assert!(m.current_schema().is_some());
    }

    #[test]
    fn current_schema_returns_none_when_id_missing() {
        let mut m = meta();
        m.current_schema_id = 99;
        assert!(m.current_schema().is_none());
    }

    #[test]
    fn default_spec_returns_match() {
        let m = meta();
        assert!(m.default_spec().is_some());
    }

    #[test]
    fn current_snapshot_none_when_no_snapshots() {
        assert!(meta().current_snapshot().is_none());
    }

    // ── add_snapshot ───────────────────────────────────────────────────────────

    #[test]
    fn add_snapshot_updates_current_and_last_seq() {
        let mut m = meta();
        let s = Snapshot::new(1, 1, 1_700_000_000_000, "/lake/m.avro", SnapshotOperation::Append);
        m.add_snapshot(s).unwrap();
        assert_eq!(m.current_snapshot_id, Some(1));
        assert_eq!(m.last_sequence_number, 1);
        assert_eq!(m.snapshots.len(), 1);
    }

    #[test]
    fn add_snapshot_must_have_increasing_seq() {
        let mut m = meta();
        let s1 = Snapshot::new(1, 5, 1_700_000_000_000, "/lake/m1.avro", SnapshotOperation::Append);
        m.add_snapshot(s1).unwrap();
        // next snapshot with seq 4 must fail
        let s2 = Snapshot::new(2, 4, 1_700_000_000_001, "/lake/m2.avro", SnapshotOperation::Append);
        let e = m.add_snapshot(s2).unwrap_err().to_string();
        assert!(e.contains("sequence_number"));
    }

    #[test]
    fn add_snapshot_first_seq_zero_allowed() {
        // last_sequence_number is 0 initially; first snapshot can have seq 0
        let mut m = meta();
        let s = Snapshot::new(1, 0, 1, "/lake/m.avro", SnapshotOperation::Append);
        assert!(m.add_snapshot(s).is_ok());
    }

    #[test]
    fn add_snapshot_tenant_mismatch_err() {
        let mut m = meta().with_tenant("acme");
        let s = Snapshot::new(1, 1, 1, "/lake/m.avro", SnapshotOperation::Append)
            .with_tenant("burak");
        let e = m.add_snapshot(s).unwrap_err().to_string();
        assert!(e.contains("tenant"));
    }

    #[test]
    fn add_snapshot_invalid_snapshot_err() {
        let mut m = meta();
        let mut s = Snapshot::new(1, 1, 1, "/lake/m.avro", SnapshotOperation::Append);
        s.snapshot_id = 0; // invalid
        assert!(m.add_snapshot(s).is_err());
    }

    // ── validate ──────────────────────────────────────────────────────────────

    #[test]
    fn validate_default_ok() {
        assert!(meta().validate().is_ok());
    }

    #[test]
    fn validate_unknown_format_version_err() {
        let mut m = meta();
        m.format_version = 99;
        assert!(m.validate().is_err());
    }

    #[test]
    fn validate_format_version_1_accepted() {
        let mut m = meta();
        m.format_version = 1;
        // v1 still allowed for legacy
        assert!(m.validate().is_ok());
    }

    #[test]
    fn validate_empty_location_err() {
        let mut m = meta();
        m.location = "".into();
        assert!(m.validate().is_err());
    }

    #[test]
    fn validate_unknown_current_schema_id_err() {
        let mut m = meta();
        m.current_schema_id = 99;
        let e = m.validate().unwrap_err().to_string();
        assert!(e.contains("current_schema_id"));
    }

    #[test]
    fn validate_unknown_default_spec_id_err() {
        let mut m = meta();
        m.default_spec_id = 99;
        let e = m.validate().unwrap_err().to_string();
        assert!(e.contains("default_spec_id"));
    }

    #[test]
    fn validate_schema_tenant_mismatch_err() {
        let mut m = meta().with_tenant("acme");
        m.schemas[0].tenant_id = "burak".into();
        let e = m.validate().unwrap_err().to_string();
        assert!(e.contains("tenant"));
    }

    #[test]
    fn validate_invalid_tenant_err() {
        let mut m = meta();
        m.tenant_id = "BAD".into();
        assert!(m.validate().is_err());
    }

    #[test]
    fn validate_current_snapshot_id_dangling_err() {
        let mut m = meta();
        m.current_snapshot_id = Some(123);
        let e = m.validate().unwrap_err().to_string();
        assert!(e.contains("current_snapshot_id"));
    }

    // ── serde ─────────────────────────────────────────────────────────────────

    #[test]
    fn metadata_serde_round_trip() {
        let mut m = meta().with_tenant("acme");
        m.schemas[0].tenant_id = "acme".into();
        let s = Snapshot::new(1, 1, 1, "/lake/m.avro", SnapshotOperation::Append).with_tenant("acme");
        m.add_snapshot(s).unwrap();
        let j = serde_json::to_string(&m).unwrap();
        let back: TableMetadata = serde_json::from_str(&j).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn metadata_uuid_unique_per_call() {
        let m1 = meta();
        let m2 = meta();
        assert_ne!(m1.table_uuid, m2.table_uuid);
    }
}
