// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! In-memory row + value model.
//!
//! DataFusion's vectorized executor batches rows into Arrow
//! `RecordBatch`es. The cave-datafusion MVP runs a *row-at-a-time*
//! executor over a small `Value` enum — slower but dependency-free
//! and clear to reason about. Swapping to an Arrow-backed batch
//! engine is a v0.2 milestone.

use crate::schema::DataType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "v")]
pub enum Value {
    Null,
    Bool(bool),
    Int32(i32),
    Int64(i64),
    Float64(f64),
    Utf8(String),
}

impl Value {
    pub fn data_type(&self) -> DataType {
        match self {
            Self::Null => DataType::Null,
            Self::Bool(_) => DataType::Boolean,
            Self::Int32(_) => DataType::Int32,
            Self::Int64(_) => DataType::Int64,
            Self::Float64(_) => DataType::Float64,
            Self::Utf8(_) => DataType::Utf8,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int32(v) => Some(*v as i64),
            Self::Int64(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int32(v) => Some(*v as f64),
            Self::Int64(v) => Some(*v as f64),
            Self::Float64(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Utf8(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Ordering that propagates NULL as "less than" — matches Iceberg's
    /// `nulls-first` default and DataFusion's default sort.
    pub fn cmp_nulls_first(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (Self::Null, Self::Null) => Ordering::Equal,
            (Self::Null, _) => Ordering::Less,
            (_, Self::Null) => Ordering::Greater,
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Int32(a), Self::Int32(b)) => a.cmp(b),
            (Self::Int64(a), Self::Int64(b)) => a.cmp(b),
            (Self::Float64(a), Self::Float64(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Self::Utf8(a), Self::Utf8(b)) => a.cmp(b),
            (a, b) => {
                // Mixed numeric: coerce through f64.
                match (a.as_f64(), b.as_f64()) {
                    (Some(af), Some(bf)) => af.partial_cmp(&bf).unwrap_or(Ordering::Equal),
                    _ => Ordering::Equal,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Row {
    pub values: Vec<Value>,
}

impl Row {
    pub fn new(values: Vec<Value>) -> Self {
        Self { values }
    }

    pub fn get(&self, idx: usize) -> Option<&Value> {
        self.values.get(idx)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_type_classification() {
        assert_eq!(Value::Bool(true).data_type(), DataType::Boolean);
        assert_eq!(Value::Int64(1).data_type(), DataType::Int64);
        assert_eq!(Value::Utf8("a".into()).data_type(), DataType::Utf8);
    }

    #[test]
    fn cmp_nulls_first_orders_null_low() {
        use std::cmp::Ordering;
        assert_eq!(Value::Null.cmp_nulls_first(&Value::Int64(1)), Ordering::Less);
        assert_eq!(Value::Int64(1).cmp_nulls_first(&Value::Null), Ordering::Greater);
        assert_eq!(Value::Null.cmp_nulls_first(&Value::Null), Ordering::Equal);
    }

    #[test]
    fn cmp_mixed_numeric_coerces() {
        use std::cmp::Ordering;
        assert_eq!(Value::Int64(1).cmp_nulls_first(&Value::Float64(1.0)), Ordering::Equal);
        assert_eq!(Value::Int64(1).cmp_nulls_first(&Value::Float64(2.0)), Ordering::Less);
    }

    #[test]
    fn as_helpers_only_match_compatible() {
        assert_eq!(Value::Int64(7).as_i64(), Some(7));
        assert_eq!(Value::Int32(7).as_i64(), Some(7));
        assert_eq!(Value::Float64(1.0).as_i64(), None);
        assert_eq!(Value::Utf8("a".into()).as_str(), Some("a"));
        assert_eq!(Value::Int64(1).as_str(), None);
    }
}
