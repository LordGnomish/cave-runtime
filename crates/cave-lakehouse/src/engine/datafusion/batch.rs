//! Minimal RecordBatch — column-major rows for the execution operators.
//!
//! Mirrors apache/arrow-rs `RecordBatch` and apache/datafusion's use of it
//! as the unit of streaming between physical operators.

use crate::engine::datafusion::error::{DataFusionError, DfResult};
use crate::engine::datafusion::tenant::{default_tenant_id, validate_tenant_id};
use serde::{Deserialize, Serialize};

/// A single column value. Subset of arrow scalar types — int64, float64,
/// utf8, bool, null. Sufficient for the operators shipped here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int64(i64),
    Float64(f64),
    Utf8(String),
}

impl Value {
    pub const fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int64(_) => "int64",
            Value::Float64(_) => "float64",
            Value::Utf8(_) => "utf8",
        }
    }

    pub fn as_int64(&self) -> Option<i64> {
        if let Value::Int64(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Value::Utf8(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// Column-major batch — schema name list + parallel `Vec<Value>` columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordBatch {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    #[serde(default = "default_tenant_id")]
    pub tenant_id: String,
}

impl RecordBatch {
    pub fn new(columns: Vec<String>, rows: Vec<Vec<Value>>) -> DfResult<Self> {
        let arity = columns.len();
        for (i, row) in rows.iter().enumerate() {
            if row.len() != arity {
                return Err(DataFusionError::Plan(format!(
                    "row {} has {} values but batch arity is {}",
                    i,
                    row.len(),
                    arity
                )));
            }
        }
        Ok(Self {
            columns,
            rows,
            tenant_id: default_tenant_id(),
        })
    }

    pub fn empty(columns: Vec<String>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            tenant_id: default_tenant_id(),
        }
    }

    pub fn with_tenant(mut self, t: impl Into<String>) -> Self {
        self.tenant_id = t.into();
        self
    }

    pub fn num_rows(&self) -> usize {
        self.rows.len()
    }

    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }

    pub fn column_index(&self, name: &str) -> DfResult<usize> {
        self.columns
            .iter()
            .position(|c| c == name)
            .ok_or_else(|| DataFusionError::ColumnNotFound(name.to_string()))
    }

    pub fn validate(&self) -> DfResult<()> {
        validate_tenant_id(&self.tenant_id)?;
        let arity = self.columns.len();
        for (i, row) in self.rows.iter().enumerate() {
            if row.len() != arity {
                return Err(DataFusionError::Plan(format!(
                    "row {} has {} values but batch arity is {}",
                    i,
                    row.len(),
                    arity
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch() -> RecordBatch {
        RecordBatch::new(
            vec!["id".into(), "name".into()],
            vec![
                vec![Value::Int64(1), Value::Utf8("alice".into())],
                vec![Value::Int64(2), Value::Utf8("bob".into())],
            ],
        )
        .unwrap()
    }

    // ── Value helpers ─────────────────────────────────────────────────────────

    #[test]
    fn value_type_names() {
        assert_eq!(Value::Null.type_name(), "null");
        assert_eq!(Value::Bool(true).type_name(), "bool");
        assert_eq!(Value::Int64(1).type_name(), "int64");
        assert_eq!(Value::Float64(1.0).type_name(), "float64");
        assert_eq!(Value::Utf8("x".into()).type_name(), "utf8");
    }

    #[test]
    fn value_as_int64() {
        assert_eq!(Value::Int64(42).as_int64(), Some(42));
        assert_eq!(Value::Bool(true).as_int64(), None);
    }

    #[test]
    fn value_as_bool() {
        assert_eq!(Value::Bool(true).as_bool(), Some(true));
        assert_eq!(Value::Int64(1).as_bool(), None);
    }

    #[test]
    fn value_as_str() {
        assert_eq!(Value::Utf8("x".into()).as_str(), Some("x"));
        assert_eq!(Value::Int64(1).as_str(), None);
    }

    #[test]
    fn value_is_null() {
        assert!(Value::Null.is_null());
        assert!(!Value::Int64(0).is_null());
    }

    // ── RecordBatch constructors ──────────────────────────────────────────────

    #[test]
    fn batch_new_arity_mismatch_err() {
        let r = RecordBatch::new(
            vec!["a".into(), "b".into()],
            vec![vec![Value::Int64(1)]], // wrong arity
        );
        assert!(r.is_err());
    }

    #[test]
    fn batch_empty() {
        let b = RecordBatch::empty(vec!["x".into()]);
        assert_eq!(b.num_rows(), 0);
        assert_eq!(b.num_columns(), 1);
    }

    #[test]
    fn batch_default_tenant() {
        assert_eq!(batch().tenant_id, "default");
    }

    #[test]
    fn batch_with_tenant() {
        let b = batch().with_tenant("acme");
        assert_eq!(b.tenant_id, "acme");
    }

    // ── lookup ────────────────────────────────────────────────────────────────

    #[test]
    fn batch_column_index_found() {
        assert_eq!(batch().column_index("name").unwrap(), 1);
    }

    #[test]
    fn batch_column_index_missing_err() {
        let e = batch().column_index("missing").unwrap_err();
        assert!(matches!(e, DataFusionError::ColumnNotFound(_)));
    }

    // ── validate ──────────────────────────────────────────────────────────────

    #[test]
    fn batch_validate_default_ok() {
        assert!(batch().validate().is_ok());
    }

    #[test]
    fn batch_validate_invalid_tenant_err() {
        let mut b = batch();
        b.tenant_id = "BAD".into();
        assert!(b.validate().is_err());
    }

    // ── serde ─────────────────────────────────────────────────────────────────

    #[test]
    fn batch_serde_roundtrip() {
        let b = batch().with_tenant("acme");
        let j = serde_json::to_string(&b).unwrap();
        let back: RecordBatch = serde_json::from_str(&j).unwrap();
        assert_eq!(back, b);
    }
}
