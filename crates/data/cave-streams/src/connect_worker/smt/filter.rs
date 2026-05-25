// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/Filter.java
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/predicates/*.java

//! Filter SMT — drop records (returning `None`) by predicate.
//!
//! Upstream's `Filter` is paired with a separate "predicate" config
//! (`HasHeaderKey`, `RecordIsTombstone`, `TopicNameMatches`) and a
//! `negate` flag. cave-streams collapses both into one SMT body for
//! the in-process port: the SMT carries the predicate kind and a
//! pattern string.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{StreamsError, StreamsResult};

use super::{RecordEnvelope, Smt, SmtRegistry, Value};

#[derive(Debug, Clone)]
pub enum FilterPredicate {
    /// Drop records whose topic exactly matches `pattern` (no
    /// regex — Kafka uses java.util.regex.Pattern but cave-streams
    /// ships glob-free exact-match; regex is tracked for follow-up).
    TopicNameMatches(String),
    /// Drop records with a tombstone value (Value::Null).
    RecordIsTombstone,
    /// Drop records that have a header with `header_key`.
    HasHeaderKey(String),
}

#[derive(Debug, Clone)]
pub struct Filter {
    pub predicate: FilterPredicate,
    /// If true, *keep* matching records; otherwise drop them.
    pub negate: bool,
}

impl Filter {
    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let pred_type = cfg
            .get("predicate")
            .map(|s| s.as_str())
            .unwrap_or("RecordIsTombstone");
        let predicate = match pred_type {
            "RecordIsTombstone" => FilterPredicate::RecordIsTombstone,
            "TopicNameMatches" => {
                let p = cfg.get("pattern").cloned().ok_or_else(|| {
                    StreamsError::Internal("Filter[TopicNameMatches]: 'pattern' is required".into())
                })?;
                FilterPredicate::TopicNameMatches(p)
            }
            "HasHeaderKey" => {
                let k = cfg.get("header.key").cloned().ok_or_else(|| {
                    StreamsError::Internal("Filter[HasHeaderKey]: 'header.key' is required".into())
                })?;
                FilterPredicate::HasHeaderKey(k)
            }
            other => {
                return Err(StreamsError::Internal(format!(
                    "Filter: unknown predicate '{other}'"
                )));
            }
        };
        let negate = cfg
            .get("negate")
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Ok(Self { predicate, negate })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.Filter$Value",
            Self::builder,
        );
        reg.register("Filter$Value", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }

    fn matches(&self, r: &RecordEnvelope) -> bool {
        match &self.predicate {
            FilterPredicate::TopicNameMatches(p) => r.topic == *p,
            FilterPredicate::RecordIsTombstone => matches!(r.value, Value::Null),
            FilterPredicate::HasHeaderKey(k) => r.headers.contains_key(k),
        }
    }
}

impl Smt for Filter {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.Filter$Value"
    }

    fn apply(&self, r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        // Default semantics: matches() → drop; with negate → keep matches only.
        let matched = self.matches(&r);
        let drop = if self.negate { !matched } else { matched };
        if drop { Ok(None) } else { Ok(Some(r)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_predicate_is_tombstone() {
        let f = Filter::from_config(&BTreeMap::new()).unwrap();
        let r_tomb = RecordEnvelope::new("t", Value::Null);
        let r_live = RecordEnvelope::new("t", Value::Int(1));
        assert!(f.apply(r_tomb).unwrap().is_none());
        assert!(f.apply(r_live).unwrap().is_some());
    }

    #[test]
    fn topic_match_drops_specific_topic() {
        let mut cfg = BTreeMap::new();
        cfg.insert("predicate".into(), "TopicNameMatches".into());
        cfg.insert("pattern".into(), "junk".into());
        let f = Filter::from_config(&cfg).unwrap();
        assert!(
            f.apply(RecordEnvelope::new("junk", Value::Int(1)))
                .unwrap()
                .is_none()
        );
        assert!(
            f.apply(RecordEnvelope::new("keep", Value::Int(1)))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn negate_inverts_match_semantics() {
        let mut cfg = BTreeMap::new();
        cfg.insert("predicate".into(), "TopicNameMatches".into());
        cfg.insert("pattern".into(), "keep".into());
        cfg.insert("negate".into(), "true".into());
        let f = Filter::from_config(&cfg).unwrap();
        // With negate: keep records that match the pattern, drop the rest.
        assert!(
            f.apply(RecordEnvelope::new("keep", Value::Int(1)))
                .unwrap()
                .is_some()
        );
        assert!(
            f.apply(RecordEnvelope::new("drop-me", Value::Int(1)))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn has_header_key_filters_on_header() {
        let mut cfg = BTreeMap::new();
        cfg.insert("predicate".into(), "HasHeaderKey".into());
        cfg.insert("header.key".into(), "trace-id".into());
        let f = Filter::from_config(&cfg).unwrap();
        let mut r = RecordEnvelope::new("t", Value::Int(1));
        r.headers.insert("trace-id".into(), vec![1]);
        assert!(f.apply(r).unwrap().is_none());
        let r2 = RecordEnvelope::new("t", Value::Int(1));
        assert!(f.apply(r2).unwrap().is_some());
    }

    #[test]
    fn missing_pattern_for_topic_match_errors() {
        let mut cfg = BTreeMap::new();
        cfg.insert("predicate".into(), "TopicNameMatches".into());
        assert!(Filter::from_config(&cfg).is_err());
    }

    #[test]
    fn unknown_predicate_errors() {
        let mut cfg = BTreeMap::new();
        cfg.insert("predicate".into(), "Bogus".into());
        assert!(Filter::from_config(&cfg).is_err());
    }
}
