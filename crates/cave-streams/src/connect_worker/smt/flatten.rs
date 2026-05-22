// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/Flatten.java

//! Flatten SMT — recursively flatten a nested object value into a
//! single-level map keyed by `delim`-joined paths. Mirrors upstream
//! `Flatten$Value`. Default delimiter `.` matches upstream's
//! `FLATTEN_DELIMITER_DEFAULT`. Scalar (non-object) values pass
//! through unchanged.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::StreamsResult;

use super::{RecordEnvelope, Smt, SmtRegistry, Value};

const DEFAULT_DELIMITER: &str = ".";

#[derive(Debug, Clone)]
pub struct Flatten {
    pub delimiter: String,
}

impl Flatten {
    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let delim = cfg
            .get("delimiter")
            .cloned()
            .unwrap_or_else(|| DEFAULT_DELIMITER.to_string());
        Ok(Self { delimiter: delim })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.Flatten$Value",
            Self::builder,
        );
        reg.register("Flatten$Value", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }
}

impl Smt for Flatten {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.Flatten$Value"
    }

    fn apply(&self, mut r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        if !r.value.is_object() {
            return Ok(Some(r));
        }
        let mut out = BTreeMap::new();
        let value = std::mem::replace(&mut r.value, Value::Null);
        if let Value::Object(m) = value {
            for (k, v) in m {
                flatten_into(&mut out, &k, v, &self.delimiter);
            }
        }
        r.value = Value::Object(out);
        Ok(Some(r))
    }
}

fn flatten_into(out: &mut BTreeMap<String, Value>, prefix: &str, value: Value, delim: &str) {
    if let Value::Object(m) = value {
        for (k, v) in m {
            let key = format!("{prefix}{delim}{k}");
            flatten_into(out, &key, v, delim);
        }
    } else {
        out.insert(prefix.to_string(), value);
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
    fn flatten_one_level_no_change() {
        let s = Flatten::from_config(&BTreeMap::new()).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("a", Value::Int(1)), ("b", Value::Bool(true))]));
        let out = s.apply(r).unwrap().unwrap();
        let m = out.value.as_object().unwrap();
        assert_eq!(m.get("a"), Some(&Value::Int(1)));
        assert_eq!(m.get("b"), Some(&Value::Bool(true)));
    }

    #[test]
    fn flatten_recursive_with_default_delim() {
        let s = Flatten::from_config(&BTreeMap::new()).unwrap();
        let r = RecordEnvelope::new(
            "t",
            obj(&[(
                "user",
                obj(&[
                    ("name", Value::String("alice".into())),
                    ("addr", obj(&[("city", Value::String("Ist".into()))])),
                ]),
            )]),
        );
        let out = s.apply(r).unwrap().unwrap();
        let m = out.value.as_object().unwrap();
        assert_eq!(m.get("user.name"), Some(&Value::String("alice".into())));
        assert_eq!(m.get("user.addr.city"), Some(&Value::String("Ist".into())));
    }

    #[test]
    fn flatten_scalar_value_passthrough() {
        let s = Flatten::from_config(&BTreeMap::new()).unwrap();
        let r = RecordEnvelope::new("t", Value::Int(7));
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(out.value, Value::Int(7));
    }
}
