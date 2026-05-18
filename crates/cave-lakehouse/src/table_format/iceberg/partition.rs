// SPDX-License-Identifier: AGPL-3.0-or-later
//! Iceberg PartitionSpec — partition transforms bound to source field ids.
//!
//! Mirrors apache/iceberg-rust crates/iceberg/src/spec/partition.rs and
//! the spec at https://iceberg.apache.org/spec/#partition-transforms.

use crate::table_format::iceberg::error::{IcebergError, IcebergResult};
use crate::table_format::iceberg::schema::Schema;
use serde::{Deserialize, Serialize};

/// Partition transforms shipped here — identity, bucket, truncate, year,
/// month, day, hour, void.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transform {
    Identity,
    Bucket(u32),
    Truncate(u32),
    Year,
    Month,
    Day,
    Hour,
    Void,
}

impl Transform {
    /// Iceberg-spec textual form ("identity", "bucket[16]", "truncate[10]", …).
    pub fn spec_repr(&self) -> String {
        match self {
            Transform::Identity => "identity".to_string(),
            Transform::Bucket(n) => format!("bucket[{}]", n),
            Transform::Truncate(n) => format!("truncate[{}]", n),
            Transform::Year => "year".to_string(),
            Transform::Month => "month".to_string(),
            Transform::Day => "day".to_string(),
            Transform::Hour => "hour".to_string(),
            Transform::Void => "void".to_string(),
        }
    }

    /// Parse the spec textual form. Returns `Err` if the string is unknown
    /// or has malformed parameters.
    pub fn parse_spec(s: &str) -> IcebergResult<Transform> {
        match s {
            "identity" => Ok(Transform::Identity),
            "year" => Ok(Transform::Year),
            "month" => Ok(Transform::Month),
            "day" => Ok(Transform::Day),
            "hour" => Ok(Transform::Hour),
            "void" => Ok(Transform::Void),
            other => {
                if let Some(rest) = other.strip_prefix("bucket[") {
                    let n_str = rest.strip_suffix(']').ok_or_else(|| {
                        IcebergError::PartitionSpec(format!(
                            "bucket transform missing ']': '{}'",
                            other
                        ))
                    })?;
                    let n: u32 = n_str.parse().map_err(|_| {
                        IcebergError::PartitionSpec(format!(
                            "bucket transform parameter must be u32: '{}'",
                            n_str
                        ))
                    })?;
                    if n == 0 {
                        return Err(IcebergError::PartitionSpec(
                            "bucket transform parameter must be > 0".into(),
                        ));
                    }
                    Ok(Transform::Bucket(n))
                } else if let Some(rest) = other.strip_prefix("truncate[") {
                    let n_str = rest.strip_suffix(']').ok_or_else(|| {
                        IcebergError::PartitionSpec(format!(
                            "truncate transform missing ']': '{}'",
                            other
                        ))
                    })?;
                    let n: u32 = n_str.parse().map_err(|_| {
                        IcebergError::PartitionSpec(format!(
                            "truncate transform parameter must be u32: '{}'",
                            n_str
                        ))
                    })?;
                    if n == 0 {
                        return Err(IcebergError::PartitionSpec(
                            "truncate transform parameter must be > 0".into(),
                        ));
                    }
                    Ok(Transform::Truncate(n))
                } else {
                    Err(IcebergError::PartitionSpec(format!(
                        "unknown transform '{}'",
                        other
                    )))
                }
            }
        }
    }
}

/// One field of a PartitionSpec.
///
/// `source_id` references a Schema field by id; `field_id` is unique within
/// the partition spec; `name` is the partition column name in writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionField {
    pub source_id: i32,
    pub field_id: i32,
    pub name: String,
    pub transform: Transform,
}

/// PartitionSpec — a list of partition fields with a unique spec id.
///
/// Mirrors apache/iceberg-rust spec/partition.rs `PartitionSpec`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionSpec {
    pub spec_id: i32,
    pub fields: Vec<PartitionField>,
}

impl PartitionSpec {
    pub fn unpartitioned(spec_id: i32) -> Self {
        Self { spec_id, fields: Vec::new() }
    }

    pub fn is_unpartitioned(&self) -> bool {
        self.fields.is_empty()
    }

    /// Validate the spec against a schema:
    /// - source_ids must reference existing schema fields
    /// - field_ids must be unique within the spec
    /// - names must be unique within the spec
    pub fn validate(&self, schema: &Schema) -> IcebergResult<()> {
        let mut seen_field_ids = std::collections::HashSet::new();
        let mut seen_names = std::collections::HashSet::new();
        for f in &self.fields {
            if schema.field_by_id(f.source_id).is_none() {
                return Err(IcebergError::PartitionSpec(format!(
                    "source_id {} not found in schema",
                    f.source_id
                )));
            }
            if !seen_field_ids.insert(f.field_id) {
                return Err(IcebergError::PartitionSpec(format!(
                    "duplicate field_id {}",
                    f.field_id
                )));
            }
            if !seen_names.insert(f.name.clone()) {
                return Err(IcebergError::PartitionSpec(format!(
                    "duplicate name '{}'",
                    f.name
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table_format::iceberg::schema::{Field, PrimitiveType, Schema};

    fn ts_schema() -> Schema {
        Schema::new(
            1,
            vec![
                Field::required(1, "id", PrimitiveType::Long),
                Field::required(2, "ts", PrimitiveType::Timestamp),
                Field::required(3, "user", PrimitiveType::String),
            ],
        )
    }

    // ── Transform spec_repr ────────────────────────────────────────────────────

    #[test]
    fn transform_spec_repr_simple() {
        // citation: iceberg spec "Partition Transforms" textual form
        assert_eq!(Transform::Identity.spec_repr(), "identity");
        assert_eq!(Transform::Year.spec_repr(), "year");
        assert_eq!(Transform::Month.spec_repr(), "month");
        assert_eq!(Transform::Day.spec_repr(), "day");
        assert_eq!(Transform::Hour.spec_repr(), "hour");
        assert_eq!(Transform::Void.spec_repr(), "void");
    }

    #[test]
    fn transform_spec_repr_parameterized() {
        assert_eq!(Transform::Bucket(16).spec_repr(), "bucket[16]");
        assert_eq!(Transform::Truncate(10).spec_repr(), "truncate[10]");
    }

    // ── Transform parse_spec — happy path ──────────────────────────────────────

    #[test]
    fn parse_identity_ok() {
        assert_eq!(Transform::parse_spec("identity").unwrap(), Transform::Identity);
    }

    #[test]
    fn parse_year_month_day_hour_void_ok() {
        assert_eq!(Transform::parse_spec("year").unwrap(), Transform::Year);
        assert_eq!(Transform::parse_spec("month").unwrap(), Transform::Month);
        assert_eq!(Transform::parse_spec("day").unwrap(), Transform::Day);
        assert_eq!(Transform::parse_spec("hour").unwrap(), Transform::Hour);
        assert_eq!(Transform::parse_spec("void").unwrap(), Transform::Void);
    }

    #[test]
    fn parse_bucket_ok() {
        assert_eq!(Transform::parse_spec("bucket[32]").unwrap(), Transform::Bucket(32));
    }

    #[test]
    fn parse_truncate_ok() {
        assert_eq!(
            Transform::parse_spec("truncate[10]").unwrap(),
            Transform::Truncate(10)
        );
    }

    #[test]
    fn parse_round_trip_all() {
        for t in [
            Transform::Identity,
            Transform::Bucket(16),
            Transform::Truncate(8),
            Transform::Year,
            Transform::Month,
            Transform::Day,
            Transform::Hour,
            Transform::Void,
        ] {
            let s = t.spec_repr();
            assert_eq!(Transform::parse_spec(&s).unwrap(), t);
        }
    }

    // ── Transform parse_spec — failures ────────────────────────────────────────

    #[test]
    fn parse_unknown_transform_err() {
        assert!(Transform::parse_spec("rotate").is_err());
    }

    #[test]
    fn parse_bucket_missing_bracket_err() {
        assert!(Transform::parse_spec("bucket[16").is_err());
    }

    #[test]
    fn parse_bucket_zero_param_err() {
        // bucket[0] is meaningless — would divide by zero
        assert!(Transform::parse_spec("bucket[0]").is_err());
    }

    #[test]
    fn parse_bucket_non_numeric_err() {
        assert!(Transform::parse_spec("bucket[abc]").is_err());
    }

    #[test]
    fn parse_truncate_zero_param_err() {
        assert!(Transform::parse_spec("truncate[0]").is_err());
    }

    // ── PartitionField serde ──────────────────────────────────────────────────

    #[test]
    fn partition_field_serde() {
        let f = PartitionField {
            source_id: 2,
            field_id: 1000,
            name: "ts_day".into(),
            transform: Transform::Day,
        };
        let j = serde_json::to_string(&f).unwrap();
        let back: PartitionField = serde_json::from_str(&j).unwrap();
        assert_eq!(back, f);
    }

    // ── PartitionSpec constructors ─────────────────────────────────────────────

    #[test]
    fn unpartitioned_spec() {
        let s = PartitionSpec::unpartitioned(0);
        assert!(s.is_unpartitioned());
        assert_eq!(s.spec_id, 0);
    }

    // ── PartitionSpec validate — happy path ────────────────────────────────────

    #[test]
    fn validate_simple_day_partition_ok() {
        let schema = ts_schema();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![PartitionField {
                source_id: 2,
                field_id: 1000,
                name: "ts_day".into(),
                transform: Transform::Day,
            }],
        };
        assert!(spec.validate(&schema).is_ok());
    }

    #[test]
    fn validate_unpartitioned_ok() {
        let schema = ts_schema();
        assert!(PartitionSpec::unpartitioned(0).validate(&schema).is_ok());
    }

    #[test]
    fn validate_multi_field_partition_ok() {
        let schema = ts_schema();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![
                PartitionField {
                    source_id: 2,
                    field_id: 1000,
                    name: "ts_day".into(),
                    transform: Transform::Day,
                },
                PartitionField {
                    source_id: 3,
                    field_id: 1001,
                    name: "user_bucket".into(),
                    transform: Transform::Bucket(16),
                },
            ],
        };
        assert!(spec.validate(&schema).is_ok());
    }

    // ── PartitionSpec validate — failures ──────────────────────────────────────

    #[test]
    fn validate_unknown_source_id_err() {
        let schema = ts_schema();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![PartitionField {
                source_id: 99,
                field_id: 1000,
                name: "x".into(),
                transform: Transform::Identity,
            }],
        };
        let e = spec.validate(&schema).unwrap_err().to_string();
        assert!(e.contains("source_id 99"));
    }

    #[test]
    fn validate_duplicate_field_id_err() {
        let schema = ts_schema();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![
                PartitionField {
                    source_id: 2,
                    field_id: 1000,
                    name: "a".into(),
                    transform: Transform::Day,
                },
                PartitionField {
                    source_id: 3,
                    field_id: 1000,
                    name: "b".into(),
                    transform: Transform::Identity,
                },
            ],
        };
        assert!(spec.validate(&schema).unwrap_err().to_string().contains("duplicate field_id"));
    }

    #[test]
    fn validate_duplicate_name_err() {
        let schema = ts_schema();
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![
                PartitionField {
                    source_id: 2,
                    field_id: 1000,
                    name: "x".into(),
                    transform: Transform::Day,
                },
                PartitionField {
                    source_id: 3,
                    field_id: 1001,
                    name: "x".into(),
                    transform: Transform::Identity,
                },
            ],
        };
        assert!(spec.validate(&schema).unwrap_err().to_string().contains("duplicate name"));
    }

    // ── PartitionSpec serde ────────────────────────────────────────────────────

    #[test]
    fn partition_spec_serde_roundtrip() {
        let spec = PartitionSpec {
            spec_id: 0,
            fields: vec![PartitionField {
                source_id: 2,
                field_id: 1000,
                name: "ts_day".into(),
                transform: Transform::Day,
            }],
        };
        let j = serde_json::to_string(&spec).unwrap();
        let back: PartitionSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(back, spec);
    }
}
