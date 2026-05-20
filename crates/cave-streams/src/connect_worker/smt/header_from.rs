// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/HeaderFrom.java

//! HeaderFrom SMT — copy or move named value-side fields into the
//! Kafka record headers.
//!
//! Mirrors upstream `HeaderFrom$Value`. Operation modes: `copy` keeps
//! the source field in the value; `move` removes it from the value
//! after copy.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{StreamsError, StreamsResult};

use super::{RecordEnvelope, Smt, SmtRegistry, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderFromOperation {
    Copy,
    Move,
}

#[derive(Debug, Clone)]
pub struct HeaderFrom {
    /// (source_field, header_name) pairs.
    pub mappings: Vec<(String, String)>,
    pub operation: HeaderFromOperation,
}

impl HeaderFrom {
    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let fields_raw = cfg
            .get("fields")
            .ok_or_else(|| StreamsError::Internal("HeaderFrom: 'fields' is required".into()))?;
        let headers_raw = cfg
            .get("headers")
            .ok_or_else(|| StreamsError::Internal("HeaderFrom: 'headers' is required".into()))?;
        let fields: Vec<String> = fields_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let headers: Vec<String> = headers_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if fields.is_empty() {
            return Err(StreamsError::Internal(
                "HeaderFrom: 'fields' yielded zero entries".into(),
            ));
        }
        if fields.len() != headers.len() {
            return Err(StreamsError::Internal(format!(
                "HeaderFrom: fields ({}) and headers ({}) must have the same length",
                fields.len(),
                headers.len()
            )));
        }
        let op = match cfg
            .get("operation")
            .map(|s| s.as_str())
            .unwrap_or("copy")
            .to_ascii_lowercase()
            .as_str()
        {
            "copy" => HeaderFromOperation::Copy,
            "move" => HeaderFromOperation::Move,
            other => {
                return Err(StreamsError::Internal(format!(
                    "HeaderFrom: bad operation '{other}' — want 'copy' or 'move'"
                )));
            }
        };
        let mappings = fields.into_iter().zip(headers).collect();
        Ok(Self {
            mappings,
            operation: op,
        })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.HeaderFrom$Value",
            Self::builder,
        );
        reg.register("HeaderFrom$Value", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }

    /// Coerce a Value into the byte form upstream uses for headers
    /// (utf-8 string of the value's JSON-ish render).
    fn header_bytes(v: &Value) -> Vec<u8> {
        match v {
            Value::Null => b"null".to_vec(),
            Value::Bool(b) => b.to_string().into_bytes(),
            Value::Int(i) => i.to_string().into_bytes(),
            Value::Float(f) => f.to_string().into_bytes(),
            Value::String(s) => s.clone().into_bytes(),
            Value::Bytes(b) => b.clone(),
            Value::Array(_) | Value::Object(_) => format!("{v:?}").into_bytes(),
        }
    }
}

impl Smt for HeaderFrom {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.HeaderFrom$Value"
    }

    fn apply(&self, mut r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        let obj = r.value.as_object_mut().ok_or_else(|| {
            StreamsError::Internal("HeaderFrom: cannot read field from non-object value".into())
        })?;
        for (field, header) in &self.mappings {
            // `move` removes; `copy` only reads.
            let snapshot = match self.operation {
                HeaderFromOperation::Move => obj.remove(field),
                HeaderFromOperation::Copy => obj.get(field).cloned(),
            };
            if let Some(v) = snapshot {
                r.headers.insert(header.clone(), Self::header_bytes(&v));
            }
        }
        Ok(Some(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(fields: &str, headers: &str, op: &str) -> BTreeMap<String, String> {
        let mut c = BTreeMap::new();
        c.insert("fields".into(), fields.into());
        c.insert("headers".into(), headers.into());
        c.insert("operation".into(), op.into());
        c
    }

    fn obj(kvs: &[(&str, Value)]) -> Value {
        let mut m = BTreeMap::new();
        for (k, v) in kvs {
            m.insert((*k).to_string(), v.clone());
        }
        Value::Object(m)
    }

    #[test]
    fn copy_keeps_source_field() {
        let s = HeaderFrom::from_config(&cfg("trace_id", "x-trace", "copy")).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("trace_id", Value::String("abc".into()))]));
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(out.headers.get("x-trace"), Some(&b"abc".to_vec()));
        // source still there
        assert!(out.value.as_object().unwrap().contains_key("trace_id"));
    }

    #[test]
    fn move_removes_source_field() {
        let s = HeaderFrom::from_config(&cfg("trace_id", "x-trace", "move")).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("trace_id", Value::Int(42))]));
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(out.headers.get("x-trace"), Some(&b"42".to_vec()));
        assert!(!out.value.as_object().unwrap().contains_key("trace_id"));
    }

    #[test]
    fn multiple_fields_map_pairwise() {
        let s =
            HeaderFrom::from_config(&cfg("trace_id, span_id", "x-trace, x-span", "copy")).unwrap();
        let r = RecordEnvelope::new(
            "t",
            obj(&[
                ("trace_id", Value::String("a".into())),
                ("span_id", Value::String("b".into())),
            ]),
        );
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(out.headers.get("x-trace"), Some(&b"a".to_vec()));
        assert_eq!(out.headers.get("x-span"), Some(&b"b".to_vec()));
    }

    #[test]
    fn length_mismatch_errors() {
        let c = cfg("a, b", "x", "copy");
        assert!(HeaderFrom::from_config(&c).is_err());
    }

    #[test]
    fn unknown_operation_errors() {
        let c = cfg("a", "x", "swap");
        assert!(HeaderFrom::from_config(&c).is_err());
    }

    #[test]
    fn missing_field_is_skipped() {
        let s = HeaderFrom::from_config(&cfg("absent", "x-absent", "copy")).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("present", Value::Int(1))]));
        let out = s.apply(r).unwrap().unwrap();
        assert!(!out.headers.contains_key("x-absent"));
    }

    #[test]
    fn non_object_value_errors() {
        let s = HeaderFrom::from_config(&cfg("a", "x", "copy")).unwrap();
        let r = RecordEnvelope::new("t", Value::Int(5));
        assert!(s.apply(r).is_err());
    }
}
