// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/ReplaceField.java

//! ReplaceField SMT — rename / include / exclude operations on the
//! field set of an object-valued record. Mirrors upstream
//! `ReplaceField$Value`.
//!
//! * `renames` — comma-separated `old:new` pairs. Each `old` key in
//!   the value is renamed to `new`.
//! * `exclude` — comma-separated field names to drop.
//! * `include` — comma-separated whitelist. If set, fields NOT in the
//!   list are dropped. Combined with `exclude`, the exclude list also
//!   applies on top of the include filter.
//!
//! Order of operations (matching `ReplaceFieldTest.testRenamesThenExcludes`):
//! filter (include) → exclude → rename. Renames happen last so that
//! the include/exclude lists key off the *original* field name set.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::error::{StreamsError, StreamsResult};

use super::{RecordEnvelope, Smt, SmtRegistry, Value};

#[derive(Debug, Clone, Default)]
pub struct ReplaceField {
    pub include: Option<BTreeSet<String>>,
    pub exclude: BTreeSet<String>,
    pub renames: BTreeMap<String, String>,
}

fn parse_csv(s: &str) -> BTreeSet<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_renames(s: &str) -> StreamsResult<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for pair in s.split(',') {
        let p = pair.trim();
        if p.is_empty() {
            continue;
        }
        let mut it = p.splitn(2, ':');
        let from = it.next().unwrap().trim().to_string();
        let to = it
            .next()
            .ok_or_else(|| {
                StreamsError::Internal(format!(
                    "ReplaceField: rename pair '{p}' is missing ':<new>'"
                ))
            })?
            .trim()
            .to_string();
        if from.is_empty() || to.is_empty() {
            return Err(StreamsError::Internal(format!(
                "ReplaceField: rename pair '{p}' has empty side"
            )));
        }
        out.insert(from, to);
    }
    Ok(out)
}

impl ReplaceField {
    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let include = cfg.get("include").map(|s| parse_csv(s));
        let exclude = cfg.get("exclude").map(|s| parse_csv(s)).unwrap_or_default();
        let renames = cfg
            .get("renames")
            .map(|s| parse_renames(s))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            include,
            exclude,
            renames,
        })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.ReplaceField$Value",
            Self::builder,
        );
        reg.register("ReplaceField$Value", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }
}

impl Smt for ReplaceField {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.ReplaceField$Value"
    }

    fn apply(&self, mut r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        if !r.value.is_object() {
            return Ok(Some(r));
        }
        let value = std::mem::replace(&mut r.value, Value::Null);
        if let Value::Object(m) = value {
            let mut out = BTreeMap::new();
            for (k, v) in m {
                // Include filter (if set).
                if let Some(inc) = &self.include {
                    if !inc.contains(&k) {
                        continue;
                    }
                }
                // Exclude filter.
                if self.exclude.contains(&k) {
                    continue;
                }
                // Rename.
                let new_key = self.renames.get(&k).cloned().unwrap_or(k);
                out.insert(new_key, v);
            }
            r.value = Value::Object(out);
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
    fn rename_only() {
        let mut cfg = BTreeMap::new();
        cfg.insert("renames".into(), "a:x".into());
        let s = ReplaceField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("a", Value::Int(1)), ("b", Value::Int(2))]));
        let out = s.apply(r).unwrap().unwrap();
        let m = out.value.as_object().unwrap();
        assert_eq!(m.get("x"), Some(&Value::Int(1)));
        assert_eq!(m.get("b"), Some(&Value::Int(2)));
    }

    #[test]
    fn exclude_only() {
        let mut cfg = BTreeMap::new();
        cfg.insert("exclude".into(), "a".into());
        let s = ReplaceField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("t", obj(&[("a", Value::Int(1)), ("b", Value::Int(2))]));
        let out = s.apply(r).unwrap().unwrap();
        let m = out.value.as_object().unwrap();
        assert!(m.get("a").is_none());
        assert_eq!(m.get("b"), Some(&Value::Int(2)));
    }

    #[test]
    fn include_only_keeps_listed() {
        let mut cfg = BTreeMap::new();
        cfg.insert("include".into(), "a,c".into());
        let s = ReplaceField::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new(
            "t",
            obj(&[
                ("a", Value::Int(1)),
                ("b", Value::Int(2)),
                ("c", Value::Int(3)),
            ]),
        );
        let out = s.apply(r).unwrap().unwrap();
        let m = out.value.as_object().unwrap();
        assert_eq!(m.len(), 2);
        assert!(m.contains_key("a") && m.contains_key("c"));
    }

    #[test]
    fn bad_rename_errors() {
        let mut cfg = BTreeMap::new();
        cfg.insert("renames".into(), "a".into());
        assert!(ReplaceField::from_config(&cfg).is_err());
    }
}
