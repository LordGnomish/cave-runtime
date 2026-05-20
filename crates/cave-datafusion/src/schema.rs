// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! DataFusion schema model.
//!
//! Upstream:
//! * `crates/datafusion-common/src/datafusion_arrow_schema.rs` (re-exports `arrow::datatypes::Schema`)
//!
//! DataFusion delegates the schema/datatype model to `arrow-rs`. The
//! cave-datafusion MVP carries an independent reduced model that
//! mirrors the subset needed for the LogicalPlan + DataFrame surface:
//! ten primitive types, nullable flag, and a stable column ordering.
//! Bridging to a full Arrow schema (and zero-copy RecordBatch reuse)
//! lands in lakehouse-ray-2.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    Boolean,
    Int32,
    Int64,
    Float32,
    Float64,
    Utf8,
    Date32,
    Timestamp,
    Decimal,
    Null,
}

impl DataType {
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::Int32 | Self::Int64 | Self::Float32 | Self::Float64 | Self::Decimal
        )
    }

    pub fn is_integer(self) -> bool {
        matches!(self, Self::Int32 | Self::Int64)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub data_type: DataType,
    pub nullable: bool,
}

impl Field {
    pub fn new(name: impl Into<String>, data_type: DataType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TableSchema {
    pub fields: Vec<Field>,
}

impl TableSchema {
    pub fn new(fields: Vec<Field>) -> Self {
        Self { fields }
    }

    pub fn field_with_name(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }
}

pub type SchemaRef = Arc<TableSchema>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_numeric_classifies() {
        assert!(DataType::Int64.is_numeric());
        assert!(DataType::Float64.is_numeric());
        assert!(!DataType::Utf8.is_numeric());
        assert!(!DataType::Boolean.is_numeric());
    }

    #[test]
    fn is_integer_classifies() {
        assert!(DataType::Int32.is_integer());
        assert!(DataType::Int64.is_integer());
        assert!(!DataType::Float64.is_integer());
    }

    #[test]
    fn table_schema_index_of() {
        let s = TableSchema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Utf8, true),
        ]);
        assert_eq!(s.index_of("a"), Some(0));
        assert_eq!(s.index_of("b"), Some(1));
        assert_eq!(s.index_of("c"), None);
    }

    #[test]
    fn field_with_name_returns_ref() {
        let s = TableSchema::new(vec![Field::new("x", DataType::Boolean, true)]);
        let f = s.field_with_name("x").unwrap();
        assert_eq!(f.data_type, DataType::Boolean);
        assert!(f.nullable);
    }
}
