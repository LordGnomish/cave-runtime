// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/ExtractField.java

//! ExtractField SMT — promote one named field to the whole value.
//! Mirrors upstream `ExtractField$Value` — typical use is unwrapping
//! a JDBC connector envelope.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{StreamsError, StreamsResult};

use super::{RecordEnvelope, Smt, SmtRegistry, Value};

#[derive(Debug, Clone)]
pub struct ExtractField {
    pub field: String,
}

impl ExtractField {
    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let f = cfg
            .get("field")
            .ok_or_else(|| StreamsError::Internal("ExtractField: 'field' is required".into()))?;
        if f.is_empty() {
            return Err(StreamsError::Internal(
                "ExtractField: 'field' must be non-empty".into(),
            ));
        }
        Ok(Self { field: f.clone() })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.ExtractField$Value",
            Self::builder,
        );
        reg.register("ExtractField$Value", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }
}

impl Smt for ExtractField {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.ExtractField$Value"
    }

    fn apply(&self, mut r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        let obj = r.value.as_object_mut().ok_or_else(|| {
            StreamsError::Internal(
                "ExtractField: cannot extract from non-object value".into(),
            )
        })?;
        let extracted = obj.remove(&self.field).unwrap_or(Value::Null);
        r.value = extracted;
        Ok(Some(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(kvs: &[(&str, Value)]) -> Value {
        let mut m = BTreeMap::new();
        for (k, v) in kvs {
            m.insert((*k).to_string(), v.clone());
        }
        Value::Object(m)
    }

    #[test]
    fn extract_promotes_field() {
        let mut cfg = BTreeMap::new();
        cfg.insert("field".into(), "payload".into());
        let s = ExtractField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new(
            "t",
            obj(&[
                ("payload", Value::String("hello".into())),
                ("envelope", Value::Bool(true)),
            ]),
        );
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(out.value, Value::String("hello".into()));
    }

    #[test]
    fn extract_missing_field_yields_null() {
        let mut cfg = BTreeMap::new();
        cfg.insert("field".into(), "absent".into());
        let s = ExtractField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("present", Value::Int(1))]));
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(out.value, Value::Null);
    }

    #[test]
    fn extract_non_object_value_errors() {
        let mut cfg = BTreeMap::new();
        cfg.insert("field".into(), "x".into());
        let s = ExtractField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", Value::Int(7));
        assert!(s.apply(r).is_err());
    }

    #[test]
    fn extract_missing_config_errors() {
        assert!(ExtractField::from_config(&BTreeMap::new()).is_err());
    }

    #[test]
    fn extract_empty_field_errors() {
        let mut cfg = BTreeMap::new();
        cfg.insert("field".into(), "".into());
        assert!(ExtractField::from_config(&cfg).is_err());
    }
}
