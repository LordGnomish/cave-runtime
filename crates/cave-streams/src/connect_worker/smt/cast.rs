// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/Cast.java

//! Cast SMT — coerce a named value-side field (or the whole value)
//! to a primitive type. Mirrors upstream `Cast$Value`. Schemaless
//! casts only; schema'd casts go through Avro/Protobuf when those
//! land.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{StreamsError, StreamsResult};

use super::{RecordEnvelope, Smt, SmtRegistry, Value};

/// `transforms.<n>.spec` = `field1:type,field2:type`, where `type` is
/// one of `int8|int16|int32|int64|float32|float64|boolean|string`.
/// Whole-value casts use the special field name `""` (`":int64"`).
#[derive(Debug, Clone)]
pub struct Cast {
    /// (field-path, target-type) pairs.
    pub specs: Vec<(String, CastTarget)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastTarget {
    Int8,
    Int16,
    Int32,
    Int64,
    Float32,
    Float64,
    Boolean,
    String,
}

impl CastTarget {
    fn parse(s: &str) -> StreamsResult<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "int8" => Self::Int8,
            "int16" => Self::Int16,
            "int32" => Self::Int32,
            "int64" => Self::Int64,
            "float32" => Self::Float32,
            "float64" => Self::Float64,
            "boolean" | "bool" => Self::Boolean,
            "string" => Self::String,
            other => {
                return Err(StreamsError::Internal(format!(
                    "Cast: unsupported target type '{other}'"
                )))
            }
        })
    }

    /// Bit-truncating cast for ints.
    fn cast(self, v: &Value) -> StreamsResult<Value> {
        Ok(match (self, v) {
            (Self::Int8, Value::Int(i)) => Value::Int((*i as i8) as i64),
            (Self::Int16, Value::Int(i)) => Value::Int((*i as i16) as i64),
            (Self::Int32, Value::Int(i)) => Value::Int((*i as i32) as i64),
            (Self::Int64, Value::Int(i)) => Value::Int(*i),
            (Self::Float32, Value::Int(i)) => Value::Float((*i as f32) as f64),
            (Self::Float64, Value::Int(i)) => Value::Float(*i as f64),
            (Self::Int8, Value::Float(f)) => Value::Int((*f as i8) as i64),
            (Self::Int16, Value::Float(f)) => Value::Int((*f as i16) as i64),
            (Self::Int32, Value::Float(f)) => Value::Int((*f as i32) as i64),
            (Self::Int64, Value::Float(f)) => Value::Int(*f as i64),
            (Self::Float32, Value::Float(f)) => Value::Float((*f as f32) as f64),
            (Self::Float64, Value::Float(f)) => Value::Float(*f),
            (Self::Boolean, Value::Bool(b)) => Value::Bool(*b),
            (Self::Boolean, Value::Int(i)) => Value::Bool(*i != 0),
            (Self::Boolean, Value::String(s)) => {
                let low = s.trim().to_ascii_lowercase();
                Value::Bool(matches!(low.as_str(), "true" | "1" | "yes"))
            }
            (Self::String, Value::Int(i)) => Value::String(i.to_string()),
            (Self::String, Value::Float(f)) => Value::String(f.to_string()),
            (Self::String, Value::Bool(b)) => Value::String(b.to_string()),
            (Self::String, Value::String(s)) => Value::String(s.clone()),
            // String→int family
            (Self::Int8 | Self::Int16 | Self::Int32 | Self::Int64, Value::String(s)) => {
                let parsed: i64 = s.trim().parse().map_err(|_| {
                    StreamsError::Internal(format!("Cast: cannot parse '{s}' as integer"))
                })?;
                // Outer match arm pins self to one of Int{8,16,32,64};
                // the catch-all is logically dead but spelled as a
                // safe pass-through to honour the workspace-wide
                // no-`unreachable!` rule in production code.
                let narrowed = match self {
                    Self::Int8 => (parsed as i8) as i64,
                    Self::Int16 => (parsed as i16) as i64,
                    Self::Int32 => (parsed as i32) as i64,
                    Self::Int64 => parsed,
                    _ => parsed,
                };
                Value::Int(narrowed)
            }
            (Self::Float32 | Self::Float64, Value::String(s)) => {
                let parsed: f64 = s.trim().parse().map_err(|_| {
                    StreamsError::Internal(format!("Cast: cannot parse '{s}' as float"))
                })?;
                Value::Float(if matches!(self, Self::Float32) {
                    (parsed as f32) as f64
                } else {
                    parsed
                })
            }
            (_, Value::Null) => Value::Null,
            (t, v) => {
                return Err(StreamsError::Internal(format!(
                    "Cast: incompatible cast {v:?} → {t:?}"
                )))
            }
        })
    }
}

impl Cast {
    /// Parse `field1:type,field2:type` into a sorted spec list.
    pub fn parse_spec(spec: &str) -> StreamsResult<Vec<(String, CastTarget)>> {
        let mut out = Vec::new();
        for piece in spec.split(',') {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            let (f, t) = piece.split_once(':').ok_or_else(|| {
                StreamsError::Internal(format!("Cast: bad spec '{piece}', want field:type"))
            })?;
            out.push((f.trim().to_string(), CastTarget::parse(t)?));
        }
        Ok(out)
    }

    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let spec = cfg
            .get("spec")
            .ok_or_else(|| StreamsError::Internal("Cast: 'spec' is required".into()))?;
        Ok(Self {
            specs: Self::parse_spec(spec)?,
        })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.Cast$Value",
            Self::builder,
        );
        reg.register("Cast$Value", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }
}

impl Smt for Cast {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.Cast$Value"
    }

    fn apply(&self, mut r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        for (field, target) in &self.specs {
            if field.is_empty() {
                // whole-value cast
                r.value = target.cast(&r.value)?;
            } else {
                let obj = r.value.as_object_mut().ok_or_else(|| {
                    StreamsError::Internal(
                        "Cast: cannot cast field on non-object value".into(),
                    )
                })?;
                if let Some(existing) = obj.get(field) {
                    let casted = target.cast(existing)?;
                    obj.insert(field.clone(), casted);
                }
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
    fn parse_spec_handles_multiple_fields() {
        let s = Cast::parse_spec("a:int32, b:string, c:boolean").unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].1, CastTarget::Int32);
        assert_eq!(s[1].1, CastTarget::String);
        assert_eq!(s[2].1, CastTarget::Boolean);
    }

    #[test]
    fn parse_spec_rejects_bad_segment() {
        assert!(Cast::parse_spec("a:int32,bogus").is_err());
    }

    #[test]
    fn parse_spec_rejects_unknown_type() {
        assert!(Cast::parse_spec("a:unknown").is_err());
    }

    #[test]
    fn cast_string_to_int_round_trips() {
        let mut cfg = BTreeMap::new();
        cfg.insert("spec".into(), "amount:int64".into());
        let c = Cast::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new(
            "t",
            obj(&[("amount", Value::String("42".into()))]),
        );
        let out = c.apply(r).unwrap().unwrap();
        assert_eq!(
            out.value.as_object().unwrap().get("amount"),
            Some(&Value::Int(42))
        );
    }

    #[test]
    fn cast_int_to_string_renders() {
        let mut cfg = BTreeMap::new();
        cfg.insert("spec".into(), "id:string".into());
        let c = Cast::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("id", Value::Int(123))]));
        let out = c.apply(r).unwrap().unwrap();
        assert_eq!(
            out.value.as_object().unwrap().get("id"),
            Some(&Value::String("123".into()))
        );
    }

    #[test]
    fn cast_int_to_int8_truncates() {
        let mut cfg = BTreeMap::new();
        cfg.insert("spec".into(), "x:int8".into());
        let c = Cast::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("x", Value::Int(257))]));
        let out = c.apply(r).unwrap().unwrap();
        // 257 → 1 mod 256 (i8 wrap).
        assert_eq!(
            out.value.as_object().unwrap().get("x"),
            Some(&Value::Int(1))
        );
    }

    #[test]
    fn cast_missing_field_is_skipped() {
        let mut cfg = BTreeMap::new();
        cfg.insert("spec".into(), "ghost:int64".into());
        let c = Cast::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("present", Value::Int(1))]));
        let out = c.apply(r).unwrap().unwrap();
        // No change — ghost field absent.
        assert_eq!(
            out.value.as_object().unwrap().get("present"),
            Some(&Value::Int(1))
        );
        assert!(out.value.as_object().unwrap().get("ghost").is_none());
    }

    #[test]
    fn cast_non_object_value_with_named_field_errors() {
        let mut cfg = BTreeMap::new();
        cfg.insert("spec".into(), "x:int64".into());
        let c = Cast::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", Value::Int(5));
        assert!(c.apply(r).is_err());
    }

    #[test]
    fn cast_whole_value_works_with_empty_field() {
        let mut cfg = BTreeMap::new();
        cfg.insert("spec".into(), ":int64".into());
        let c = Cast::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", Value::String("99".into()));
        let out = c.apply(r).unwrap().unwrap();
        assert_eq!(out.value, Value::Int(99));
    }

    #[test]
    fn cast_string_to_boolean_yes_no() {
        let mut cfg = BTreeMap::new();
        cfg.insert("spec".into(), "ok:boolean".into());
        let c = Cast::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("ok", Value::String("yes".into()))]));
        let out = c.apply(r).unwrap().unwrap();
        assert_eq!(
            out.value.as_object().unwrap().get("ok"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn cast_missing_spec_errors() {
        let c = Cast::from_config(&BTreeMap::new());
        assert!(c.is_err());
    }
}
