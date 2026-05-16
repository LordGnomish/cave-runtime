// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/InsertField.java

//! InsertField SMT — inject a constant value or a metadata field
//! (topic / partition / offset / timestamp) into the value-side
//! object. Mirrors upstream `InsertField$Value`.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{StreamsError, StreamsResult};

use super::{RecordEnvelope, Smt, SmtRegistry, Value};

#[derive(Debug, Clone)]
pub struct InsertField {
    /// (field-name, source) injected entries.
    pub inserts: Vec<(String, InsertSource)>,
}

#[derive(Debug, Clone)]
pub enum InsertSource {
    /// Static literal value.
    Static(Value),
    /// Record topic.
    Topic,
    /// Record partition (null on missing).
    Partition,
    /// Record offset (null on missing).
    Offset,
    /// Record timestamp_ms (null on missing).
    Timestamp,
}

impl InsertField {
    /// Recognised config keys in upstream Java:
    ///   * `static.field` + `static.value` — literal injection
    ///   * `topic.field`, `partition.field`, `offset.field`,
    ///     `timestamp.field` — metadata injection
    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let mut inserts = Vec::new();
        if let Some(name) = cfg.get("static.field") {
            let v = cfg.get("static.value").cloned().unwrap_or_default();
            inserts.push((name.clone(), InsertSource::Static(Value::String(v))));
        }
        if let Some(name) = cfg.get("topic.field") {
            inserts.push((name.clone(), InsertSource::Topic));
        }
        if let Some(name) = cfg.get("partition.field") {
            inserts.push((name.clone(), InsertSource::Partition));
        }
        if let Some(name) = cfg.get("offset.field") {
            inserts.push((name.clone(), InsertSource::Offset));
        }
        if let Some(name) = cfg.get("timestamp.field") {
            inserts.push((name.clone(), InsertSource::Timestamp));
        }
        if inserts.is_empty() {
            return Err(StreamsError::Internal(
                "InsertField: at least one of static.field/topic.field/partition.field/offset.field/timestamp.field is required".into(),
            ));
        }
        Ok(Self { inserts })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.InsertField$Value",
            Self::builder,
        );
        reg.register("InsertField$Value", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }
}

impl Smt for InsertField {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.InsertField$Value"
    }

    fn apply(&self, mut r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        // If value is Null, promote to an empty object so we can
        // insert — matches upstream's auto-wrap.
        if matches!(r.value, Value::Null) {
            r.value = Value::Object(BTreeMap::new());
        }
        let obj = r.value.as_object_mut().ok_or_else(|| {
            StreamsError::Internal(
                "InsertField: cannot inject into non-object value".into(),
            )
        })?;
        for (field, src) in &self.inserts {
            let v = match src {
                InsertSource::Static(v) => v.clone(),
                InsertSource::Topic => Value::String(r.topic.clone()),
                InsertSource::Partition => match r.partition {
                    Some(p) => Value::Int(p as i64),
                    None => Value::Null,
                },
                InsertSource::Offset => match r.kafka_offset {
                    Some(o) => Value::Int(o as i64),
                    None => Value::Null,
                },
                InsertSource::Timestamp => match r.timestamp_ms {
                    Some(t) => Value::Int(t),
                    None => Value::Null,
                },
            };
            obj.insert(field.clone(), v);
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
    fn insert_static_field_adds_kv() {
        let mut cfg = BTreeMap::new();
        cfg.insert("static.field".into(), "tag".into());
        cfg.insert("static.value".into(), "v1".into());
        let s = InsertField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("a", Value::Int(1))]));
        let out = s.apply(r).unwrap().unwrap();
        let map = out.value.as_object().unwrap();
        assert_eq!(map.get("tag"), Some(&Value::String("v1".into())));
        assert_eq!(map.get("a"), Some(&Value::Int(1)));
    }

    #[test]
    fn insert_topic_field_uses_topic() {
        let mut cfg = BTreeMap::new();
        cfg.insert("topic.field".into(), "_topic".into());
        let s = InsertField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("orders", obj(&[]));
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(
            out.value.as_object().unwrap().get("_topic"),
            Some(&Value::String("orders".into()))
        );
    }

    #[test]
    fn insert_partition_and_offset_use_metadata() {
        let mut cfg = BTreeMap::new();
        cfg.insert("partition.field".into(), "_p".into());
        cfg.insert("offset.field".into(), "_o".into());
        let s = InsertField::from_config(&cfg).unwrap();
        let mut r = RecordEnvelope::new("t", obj(&[]));
        r.partition = Some(3);
        r.kafka_offset = Some(99);
        let out = s.apply(r).unwrap().unwrap();
        let m = out.value.as_object().unwrap();
        assert_eq!(m.get("_p"), Some(&Value::Int(3)));
        assert_eq!(m.get("_o"), Some(&Value::Int(99)));
    }

    #[test]
    fn missing_partition_becomes_null() {
        let mut cfg = BTreeMap::new();
        cfg.insert("partition.field".into(), "_p".into());
        let s = InsertField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[]));
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(out.value.as_object().unwrap().get("_p"), Some(&Value::Null));
    }

    #[test]
    fn null_value_becomes_object_for_injection() {
        let mut cfg = BTreeMap::new();
        cfg.insert("static.field".into(), "tag".into());
        cfg.insert("static.value".into(), "ok".into());
        let s = InsertField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", Value::Null);
        let out = s.apply(r).unwrap().unwrap();
        assert!(out.value.is_object());
        assert_eq!(
            out.value.as_object().unwrap().get("tag"),
            Some(&Value::String("ok".into()))
        );
    }

    #[test]
    fn no_inserts_configured_errors() {
        assert!(InsertField::from_config(&BTreeMap::new()).is_err());
    }

    #[test]
    fn non_object_non_null_value_errors() {
        let mut cfg = BTreeMap::new();
        cfg.insert("static.field".into(), "x".into());
        cfg.insert("static.value".into(), "y".into());
        let s = InsertField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", Value::Int(1));
        assert!(s.apply(r).is_err());
    }
}
