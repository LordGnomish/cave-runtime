// SPDX-License-Identifier: AGPL-3.0-or-later
//! LabelSelector matchExpressions — `pkg/apis/meta/v1/helpers.go::LabelSelectorAsSelector`.
//!
//! Extends [`crate::rbac::cluster_role_aggregation::LabelSelector`] (which
//! supports only `matchLabels`) with the four operators used in upstream
//! `matchExpressions[]`:
//!
//! * `In` — value of `key` must be in `values[]`.
//! * `NotIn` — value of `key` must NOT be in `values[]`.
//! * `Exists` — `key` must be present (any value); `values[]` must be empty.
//! * `DoesNotExist` — `key` must be absent; `values[]` must be empty.

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchOp {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelMatchExpression {
    pub key: String,
    pub operator: MatchOp,
    pub values: Vec<String>,
}

impl LabelMatchExpression {
    pub fn validate(&self) -> Result<(), ControllerError> {
        if self.key.is_empty() {
            return Err(ControllerError::InvalidSpec {
                kind: "LabelSelectorRequirement",
                reason: "key required".into(),
            });
        }
        match self.operator {
            MatchOp::In | MatchOp::NotIn if self.values.is_empty() => {
                Err(ControllerError::InvalidSpec {
                    kind: "LabelSelectorRequirement",
                    reason: "In/NotIn require non-empty values".into(),
                })
            }
            MatchOp::Exists | MatchOp::DoesNotExist if !self.values.is_empty() => {
                Err(ControllerError::InvalidSpec {
                    kind: "LabelSelectorRequirement",
                    reason: "Exists/DoesNotExist must have empty values".into(),
                })
            }
            _ => Ok(()),
        }
    }

    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        match self.operator {
            MatchOp::In => labels
                .get(&self.key)
                .map(|v| self.values.contains(v))
                .unwrap_or(false),
            MatchOp::NotIn => labels
                .get(&self.key)
                .map(|v| !self.values.contains(v))
                .unwrap_or(true),
            MatchOp::Exists => labels.contains_key(&self.key),
            MatchOp::DoesNotExist => !labels.contains_key(&self.key),
        }
    }
}

/// Combined `LabelSelector` with both `matchLabels` and `matchExpressions[]`.
/// Mirrors `metav1.LabelSelector`. Both halves must match (AND).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CombinedSelector {
    pub match_labels: BTreeMap<String, String>,
    pub match_expressions: Vec<LabelMatchExpression>,
}

impl CombinedSelector {
    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        for (k, v) in &self.match_labels {
            if labels.get(k) != Some(v) {
                return false;
            }
        }
        for expr in &self.match_expressions {
            if !expr.matches(labels) {
                return false;
            }
        }
        true
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/apis/meta/v1/helpers.go",
    "LabelSelectorAsSelector",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn lbl(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }
    fn expr(key: &str, op: MatchOp, vals: &[&str]) -> LabelMatchExpression {
        LabelMatchExpression {
            key: key.into(),
            operator: op,
            values: vals.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn in_op_matches_when_value_in_set() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-rbac-mexp-in-yes"
        );
        let e = expr("env", MatchOp::In, &["prod", "staging"]);
        assert!(e.matches(&lbl(&[("env", "prod")])));
    }

    #[test]
    fn in_op_no_match_when_label_missing() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-rbac-mexp-in-missing"
        );
        let e = expr("env", MatchOp::In, &["prod"]);
        assert!(!e.matches(&lbl(&[])));
    }

    #[test]
    fn not_in_matches_when_label_missing_too() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-rbac-mexp-notin-missing"
        );
        let e = expr("env", MatchOp::NotIn, &["prod"]);
        // NotIn matches when the key isn't present at all (mirrors selectorset semantics).
        assert!(e.matches(&lbl(&[])));
    }

    #[test]
    fn not_in_no_match_when_value_in_excluded() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-rbac-mexp-notin-hit"
        );
        let e = expr("env", MatchOp::NotIn, &["prod"]);
        assert!(!e.matches(&lbl(&[("env", "prod")])));
    }

    #[test]
    fn exists_matches_any_value() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-rbac-mexp-exists"
        );
        let e = expr("flag", MatchOp::Exists, &[]);
        assert!(e.matches(&lbl(&[("flag", "")])));
        assert!(!e.matches(&lbl(&[])));
    }

    #[test]
    fn does_not_exist_matches_when_absent() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-rbac-mexp-dne"
        );
        let e = expr("flag", MatchOp::DoesNotExist, &[]);
        assert!(e.matches(&lbl(&[])));
        assert!(!e.matches(&lbl(&[("flag", "x")])));
    }

    #[test]
    fn validate_in_requires_values() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/validation/validation.go",
            "ValidateLabelSelectorRequirement",
            "tenant-rbac-mexp-validate-in-empty"
        );
        let e = expr("env", MatchOp::In, &[]);
        assert!(e.validate().is_err());
    }

    #[test]
    fn validate_exists_must_have_empty_values() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/validation/validation.go",
            "ValidateLabelSelectorRequirement",
            "tenant-rbac-mexp-validate-exists-vals"
        );
        let e = expr("flag", MatchOp::Exists, &["x"]);
        assert!(e.validate().is_err());
    }

    #[test]
    fn validate_empty_key_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/validation/validation.go",
            "ValidateLabelSelectorRequirement",
            "tenant-rbac-mexp-no-key"
        );
        let e = expr("", MatchOp::Exists, &[]);
        assert!(e.validate().is_err());
    }

    #[test]
    fn combined_selector_anding() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-rbac-mexp-combined"
        );
        let s = CombinedSelector {
            match_labels: lbl(&[("env", "prod")]),
            match_expressions: vec![expr("tier", MatchOp::In, &["frontend", "backend"])],
        };
        assert!(s.matches(&lbl(&[("env", "prod"), ("tier", "frontend")])));
        // env mismatch.
        assert!(!s.matches(&lbl(&[("env", "stg"), ("tier", "frontend")])));
        // tier not in set.
        assert!(!s.matches(&lbl(&[("env", "prod"), ("tier", "data")])));
    }

    #[test]
    fn empty_combined_selector_matches_everything() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/helpers.go",
            "LabelSelectorAsSelector",
            "tenant-rbac-mexp-empty"
        );
        let s = CombinedSelector::default();
        assert!(s.matches(&lbl(&[])));
        assert!(s.matches(&lbl(&[("a", "1")])));
    }

    #[test]
    fn match_expression_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/meta/v1/types.go",
            "LabelSelectorRequirement",
            "tenant-rbac-mexp-serde"
        );
        let e = expr("env", MatchOp::In, &["a", "b"]);
        let s = serde_json::to_string(&e).unwrap();
        let back: LabelMatchExpression = serde_json::from_str(&s).unwrap();
        assert_eq!(e.key, back.key);
        assert_eq!(e.values, back.values);
    }
}
