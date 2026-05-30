// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD — Iceberg v3 column **default values** + reserved
//! row-lineage field ids.
//!
//! Upstream spec (format/spec.md, v3 additions):
//!   * A struct field may carry `initial-default` (the value backfilled
//!     for existing rows when a column is added; immutable thereafter)
//!     and `write-default` (the value used for new rows that omit the
//!     column; modifiable via schema evolution). This delivers SQL
//!     default-value semantics without rewriting data files.
//!   * Reserved metadata field ids: `_row_id` = 2147483540,
//!     `_last_updated_sequence_number` = 2147483539.
//!
//! Closes the v3-spec partial (default-row) in parity.manifest.toml.

use cave_iceberg::schema::{
    NestedField, RESERVED_FIELD_ID_LAST_UPDATED_SEQ, RESERVED_FIELD_ID_ROW_ID,
};
use cave_iceberg::{PrimitiveType, Type};
use serde_json::json;

#[test]
fn nested_field_defaults_none_by_default() {
    let f = NestedField::optional(3, "amount", Type::Primitive(PrimitiveType::Long));
    assert!(f.initial_default.is_none());
    assert!(f.write_default.is_none());
}

#[test]
fn with_initial_and_write_default_set_values() {
    let f = NestedField::optional(3, "amount", Type::Primitive(PrimitiveType::Long))
        .with_initial_default(json!(0))
        .with_write_default(json!(10));
    assert_eq!(f.initial_default, Some(json!(0)));
    assert_eq!(f.write_default, Some(json!(10)));
}

#[test]
fn read_default_prefers_initial_default() {
    // On read of an existing row that predates the column, the
    // initial-default backfills the missing value.
    let f = NestedField::optional(3, "amount", Type::Primitive(PrimitiveType::Long))
        .with_initial_default(json!(0))
        .with_write_default(json!(10));
    assert_eq!(f.read_default(), Some(&json!(0)));
}

#[test]
fn read_default_none_when_unset() {
    let f = NestedField::required(1, "id", Type::Primitive(PrimitiveType::Long));
    assert_eq!(f.read_default(), None);
}

#[test]
fn defaults_round_trip_kebab_json() {
    let f = NestedField::optional(3, "amount", Type::Primitive(PrimitiveType::Long))
        .with_initial_default(json!(7))
        .with_write_default(json!(9));
    let v = serde_json::to_value(&f).unwrap();
    assert_eq!(v["initial-default"], json!(7));
    assert_eq!(v["write-default"], json!(9));

    let back: NestedField = serde_json::from_value(v).unwrap();
    assert_eq!(back.initial_default, Some(json!(7)));
    assert_eq!(back.write_default, Some(json!(9)));
}

#[test]
fn defaults_omitted_from_json_when_unset() {
    let f = NestedField::required(1, "id", Type::Primitive(PrimitiveType::Long));
    let s = serde_json::to_string(&f).unwrap();
    assert!(!s.contains("initial-default"));
    assert!(!s.contains("write-default"));
}

#[test]
fn reserved_field_id_constants_match_spec() {
    assert_eq!(RESERVED_FIELD_ID_ROW_ID, 2147483540);
    assert_eq!(RESERVED_FIELD_ID_LAST_UPDATED_SEQ, 2147483539);
}

#[test]
fn reserved_metadata_columns_are_long_typed() {
    let row_id = NestedField::row_id();
    assert_eq!(row_id.id, RESERVED_FIELD_ID_ROW_ID);
    assert_eq!(row_id.name, "_row_id");
    assert_eq!(row_id.field_type, Type::Primitive(PrimitiveType::Long));

    let seq = NestedField::last_updated_sequence_number();
    assert_eq!(seq.id, RESERVED_FIELD_ID_LAST_UPDATED_SEQ);
    assert_eq!(seq.name, "_last_updated_sequence_number");
    assert_eq!(seq.field_type, Type::Primitive(PrimitiveType::Long));
}
