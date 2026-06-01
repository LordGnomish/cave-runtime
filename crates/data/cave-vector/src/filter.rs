// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Payload filtering.
//!
//! Port of Qdrant `lib/segment/src/types.rs` `Filter` / `Condition`. A
//! [`Filter`] is the boolean combinator `must` (AND) / `should` (OR) /
//! `must_not` (NOR) over field [`Condition`]s. An empty filter matches every
//! payload.

use crate::models::Payload;
use serde::{Deserialize, Serialize};

/// A single field condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    /// Exact JSON-value match on `key`.
    Match {
        /// Payload key.
        key: String,
        /// Required value.
        value: serde_json::Value,
    },
    /// `key` is any of `values` (set membership).
    MatchAny {
        /// Payload key.
        key: String,
        /// Allowed values.
        values: Vec<serde_json::Value>,
    },
    /// Numeric range on `key`.
    Range {
        /// Payload key.
        key: String,
        /// `>=` bound.
        #[serde(default)]
        gte: Option<f64>,
        /// `<=` bound.
        #[serde(default)]
        lte: Option<f64>,
        /// `>` bound.
        #[serde(default)]
        gt: Option<f64>,
        /// `<` bound.
        #[serde(default)]
        lt: Option<f64>,
    },
}

impl Condition {
    /// Whether this condition holds for `payload`.
    pub fn matches(&self, payload: &Payload) -> bool {
        match self {
            Condition::Match { key, value } => payload.get(key) == Some(value),
            Condition::MatchAny { key, values } => {
                payload.get(key).is_some_and(|v| values.contains(v))
            }
            Condition::Range { key, gte, lte, gt, lt } => {
                let Some(n) = payload.get(key).and_then(|v| v.as_f64()) else {
                    return false;
                };
                gte.is_none_or(|b| n >= b)
                    && lte.is_none_or(|b| n <= b)
                    && gt.is_none_or(|b| n > b)
                    && lt.is_none_or(|b| n < b)
            }
        }
    }
}

/// Boolean combinator over conditions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    /// All must hold (AND).
    #[serde(default)]
    pub must: Vec<Condition>,
    /// At least one must hold when non-empty (OR).
    #[serde(default)]
    pub should: Vec<Condition>,
    /// None may hold (NOR).
    #[serde(default)]
    pub must_not: Vec<Condition>,
}

impl Filter {
    /// Whether `payload` passes the filter.
    pub fn matches(&self, payload: &Payload) -> bool {
        let must_ok = self.must.iter().all(|c| c.matches(payload));
        let should_ok = self.should.is_empty() || self.should.iter().any(|c| c.matches(payload));
        let must_not_ok = !self.must_not.iter().any(|c| c.matches(payload));
        must_ok && should_ok && must_not_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(pairs: &[(&str, serde_json::Value)]) -> Payload {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn match_string_and_number() {
        let p = payload(&[("color", json!("red")), ("size", json!(42))]);
        assert!(Condition::Match { key: "color".into(), value: json!("red") }.matches(&p));
        assert!(!Condition::Match { key: "color".into(), value: json!("blue") }.matches(&p));
        assert!(Condition::Match { key: "size".into(), value: json!(42) }.matches(&p));
    }

    #[test]
    fn match_missing_key_is_false() {
        let p = payload(&[("color", json!("red"))]);
        assert!(!Condition::Match { key: "weight".into(), value: json!(1) }.matches(&p));
    }

    #[test]
    fn match_any_membership() {
        let p = payload(&[("color", json!("green"))]);
        let c = Condition::MatchAny {
            key: "color".into(),
            values: vec![json!("red"), json!("green"), json!("blue")],
        };
        assert!(c.matches(&p));
        let p2 = payload(&[("color", json!("pink"))]);
        assert!(!c.matches(&p2));
    }

    #[test]
    fn range_bounds() {
        let p = payload(&[("price", json!(50.0))]);
        assert!(Condition::Range { key: "price".into(), gte: Some(10.0), lte: Some(100.0), gt: None, lt: None }.matches(&p));
        assert!(!Condition::Range { key: "price".into(), gte: Some(60.0), lte: None, gt: None, lt: None }.matches(&p));
        assert!(Condition::Range { key: "price".into(), gte: None, lte: None, gt: Some(49.9), lt: Some(50.1) }.matches(&p));
        // gt is strict
        assert!(!Condition::Range { key: "price".into(), gte: None, lte: None, gt: Some(50.0), lt: None }.matches(&p));
    }

    #[test]
    fn empty_filter_matches_all() {
        let p = payload(&[("x", json!(1))]);
        assert!(Filter::default().matches(&p));
    }

    #[test]
    fn must_is_and() {
        let p = payload(&[("a", json!(1)), ("b", json!(2))]);
        let f = Filter {
            must: vec![
                Condition::Match { key: "a".into(), value: json!(1) },
                Condition::Match { key: "b".into(), value: json!(2) },
            ],
            ..Default::default()
        };
        assert!(f.matches(&p));
        let f2 = Filter {
            must: vec![
                Condition::Match { key: "a".into(), value: json!(1) },
                Condition::Match { key: "b".into(), value: json!(99) },
            ],
            ..Default::default()
        };
        assert!(!f2.matches(&p));
    }

    #[test]
    fn should_is_or() {
        let p = payload(&[("a", json!(1))]);
        let f = Filter {
            should: vec![
                Condition::Match { key: "a".into(), value: json!(1) },
                Condition::Match { key: "z".into(), value: json!(9) },
            ],
            ..Default::default()
        };
        assert!(f.matches(&p));
        let f2 = Filter {
            should: vec![Condition::Match { key: "z".into(), value: json!(9) }],
            ..Default::default()
        };
        assert!(!f2.matches(&p));
    }

    #[test]
    fn must_not_is_nor() {
        let p = payload(&[("status", json!("active"))]);
        let f = Filter {
            must_not: vec![Condition::Match { key: "status".into(), value: json!("banned") }],
            ..Default::default()
        };
        assert!(f.matches(&p));
        let f2 = Filter {
            must_not: vec![Condition::Match { key: "status".into(), value: json!("active") }],
            ..Default::default()
        };
        assert!(!f2.matches(&p));
    }
}
