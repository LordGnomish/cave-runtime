// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/MaskField.java

//! MaskField SMT — replace named value-side fields with the
//! type-appropriate "zero" or a configured replacement string.
//!
//! Mirrors upstream `MaskField$Value`. Commonly used to scrub PII
//! (`password`, `ssn`) before the record reaches the sink.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{StreamsError, StreamsResult};

use super::{RecordEnvelope, Smt, SmtRegistry, Value};

#[derive(Debug, Clone)]
pub struct MaskField {
    pub fields: Vec<String>,
    /// Replacement string. `None` → type-zero (0 / "" / false /
    /// Null), matching upstream's default behaviour.
    pub replacement: Option<String>,
}

impl MaskField {
    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let fields_raw = cfg
            .get("fields")
            .ok_or_else(|| StreamsError::Internal("MaskField: 'fields' is required".into()))?;
        let fields: Vec<String> = fields_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if fields.is_empty() {
            return Err(StreamsError::Internal(
                "MaskField: 'fields' yielded zero entries".into(),
            ));
        }
        Ok(Self {
            fields,
            replacement: cfg.get("replacement").cloned(),
        })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.MaskField$Value",
            Self::builder,
        );
        reg.register("MaskField$Value", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }

    /// Mask one value. Honors the type-zero rule when no replacement
    /// is configured; otherwise force-stringifies the replacement.
    fn masked(&self, original: &Value) -> Value {
        match &self.replacement {
            Some(r) => Value::String(r.clone()),
            None => match original {
                Value::Null => Value::Null,
                Value::Bool(_) => Value::Bool(false),
                Value::Int(_) => Value::Int(0),
                Value::Float(_) => Value::Float(0.0),
                Value::String(_) => Value::String(String::new()),
                Value::Bytes(_) => Value::Bytes(Vec::new()),
                Value::Array(_) => Value::Array(Vec::new()),
                Value::Object(_) => Value::Object(BTreeMap::new()),
            },
        }
    }
}

impl Smt for MaskField {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.MaskField$Value"
    }

    fn apply(&self, mut r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        let obj = r.value.as_object_mut().ok_or_else(|| {
            StreamsError::Internal(
                "MaskField: cannot mask field on non-object value".into(),
            )
        })?;
        for field in &self.fields {
            if let Some(existing) = obj.get(field) {
                let masked = self.masked(existing);
                obj.insert(field.clone(), masked);
            }
        }
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
    fn mask_string_default_zero_is_empty_string() {
        let mut cfg = BTreeMap::new();
        cfg.insert("fields".into(), "password".into());
        let s = MaskField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new(
            "t",
            obj(&[("password", Value::String("secret".into()))]),
        );
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(
            out.value.as_object().unwrap().get("password"),
            Some(&Value::String("".into()))
        );
    }

    #[test]
    fn mask_int_default_zero_is_zero() {
        let mut cfg = BTreeMap::new();
        cfg.insert("fields".into(), "ssn".into());
        let s = MaskField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("ssn", Value::Int(123_456_789))]));
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(
            out.value.as_object().unwrap().get("ssn"),
            Some(&Value::Int(0))
        );
    }

    #[test]
    fn mask_bool_default_zero_is_false() {
        let mut cfg = BTreeMap::new();
        cfg.insert("fields".into(), "is_admin".into());
        let s = MaskField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("is_admin", Value::Bool(true))]));
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(
            out.value.as_object().unwrap().get("is_admin"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn mask_with_replacement_uses_string() {
        let mut cfg = BTreeMap::new();
        cfg.insert("fields".into(), "password".into());
        cfg.insert("replacement".into(), "****".into());
        let s = MaskField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new(
            "t",
            obj(&[("password", Value::String("secret".into()))]),
        );
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(
            out.value.as_object().unwrap().get("password"),
            Some(&Value::String("****".into()))
        );
    }

    #[test]
    fn mask_multiple_fields_at_once() {
        let mut cfg = BTreeMap::new();
        cfg.insert("fields".into(), "a, b, c".into());
        let s = MaskField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new(
            "t",
            obj(&[
                ("a", Value::String("aa".into())),
                ("b", Value::Int(99)),
                ("c", Value::Bool(true)),
            ]),
        );
        let out = s.apply(r).unwrap().unwrap();
        let m = out.value.as_object().unwrap();
        assert_eq!(m.get("a"), Some(&Value::String("".into())));
        assert_eq!(m.get("b"), Some(&Value::Int(0)));
        assert_eq!(m.get("c"), Some(&Value::Bool(false)));
    }

    #[test]
    fn mask_missing_fields_is_skip() {
        let mut cfg = BTreeMap::new();
        cfg.insert("fields".into(), "absent".into());
        let s = MaskField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("present", Value::Int(1))]));
        let out = s.apply(r).unwrap().unwrap();
        let m = out.value.as_object().unwrap();
        assert_eq!(m.get("present"), Some(&Value::Int(1)));
        assert!(m.get("absent").is_none());
    }

    #[test]
    fn mask_non_object_value_errors() {
        let mut cfg = BTreeMap::new();
        cfg.insert("fields".into(), "x".into());
        let s = MaskField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", Value::Int(1));
        assert!(s.apply(r).is_err());
    }

    #[test]
    fn missing_fields_config_errors() {
        assert!(MaskField::from_config(&BTreeMap::new()).is_err());
    }
}
