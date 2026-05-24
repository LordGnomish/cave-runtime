// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Policy + condition vocabulary.

use crate::models::Component;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Mirrors `model/PolicyCondition.Operator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PolicyOperator {
    Is,
    IsNot,
    Matches,
    NoMatch,
    NumericEqual,
    NumericNotEqual,
    NumericLessThan,
    NumericGreaterThan,
    NumericLessThanOrEqual,
    NumericGreaterThanOrEqual,
    ContainsAll,
    ContainsAny,
}

/// Mirrors `model/PolicyCondition.Subject`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Subject {
    License,
    LicenseGroup,
    Severity,
    Cwe,
    PackageUrl,
    Cpe,
    Coordinates,
    ComponentAge,
    ComponentHash,
    Version,
    Epss,
    VulnerabilityId,
}

/// Mirrors `model/Policy.Operator` (ANY / ALL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PolicyAggregator {
    Any,
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyCondition {
    pub subject: Subject,
    pub operator: PolicyOperator,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub uuid: Uuid,
    pub name: String,
    pub aggregator: PolicyAggregator,
    pub conditions: Vec<PolicyCondition>,
    /// FAIL / WARN / INFO — mirrors `model/Policy.ViolationState`.
    pub violation_state: String,
}

impl Policy {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            name: name.into(),
            aggregator: PolicyAggregator::Any,
            conditions: Vec::new(),
            violation_state: "FAIL".into(),
        }
    }

    pub fn with_condition(mut self, c: PolicyCondition) -> Self {
        self.conditions.push(c);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViolationKind {
    License,
    Operational,
    Security,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyResult {
    pub policy: Uuid,
    pub component: Uuid,
    pub kind: ViolationKind,
    pub condition_index: usize,
    pub reason: String,
}

/// Aggregate per-condition matches into a final pass/fail.
pub fn aggregate_matches(matches: &[bool], aggregator: PolicyAggregator) -> bool {
    if matches.is_empty() {
        return false;
    }
    match aggregator {
        PolicyAggregator::All => matches.iter().all(|x| *x),
        PolicyAggregator::Any => matches.iter().any(|x| *x),
    }
}

/// Returns true when `value` matches `condition` under string-eq semantics.
pub fn check_string(condition: &str, value: &str, op: PolicyOperator) -> bool {
    match op {
        PolicyOperator::Is => value == condition,
        PolicyOperator::IsNot => value != condition,
        PolicyOperator::Matches => {
            regex::Regex::new(condition)
                .map(|r| r.is_match(value))
                .unwrap_or(false)
        }
        PolicyOperator::NoMatch => {
            regex::Regex::new(condition)
                .map(|r| !r.is_match(value))
                .unwrap_or(true)
        }
        _ => false,
    }
}

/// Returns true when `value` matches `condition` under numeric semantics.
pub fn check_numeric(condition_value: f64, value: f64, op: PolicyOperator) -> bool {
    match op {
        PolicyOperator::NumericEqual => (value - condition_value).abs() < f64::EPSILON,
        PolicyOperator::NumericNotEqual => (value - condition_value).abs() >= f64::EPSILON,
        PolicyOperator::NumericLessThan => value < condition_value,
        PolicyOperator::NumericGreaterThan => value > condition_value,
        PolicyOperator::NumericLessThanOrEqual => value <= condition_value,
        PolicyOperator::NumericGreaterThanOrEqual => value >= condition_value,
        _ => false,
    }
}

/// `false` if the component is missing the field being checked.
pub fn applies_to(condition: &PolicyCondition, c: &Component) -> bool {
    match condition.subject {
        Subject::License => c.license.is_some() || c.license_expression.is_some(),
        Subject::PackageUrl => c.purl.is_some(),
        Subject::Cpe => c.cpe.is_some(),
        Subject::ComponentHash => {
            c.md5.is_some() || c.sha1.is_some() || c.sha256.is_some() || c.sha512.is_some()
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregator_all_requires_every_true() {
        assert!(aggregate_matches(&[true, true, true], PolicyAggregator::All));
        assert!(!aggregate_matches(&[true, false, true], PolicyAggregator::All));
    }

    #[test]
    fn aggregator_any_requires_one_true() {
        assert!(aggregate_matches(&[false, true, false], PolicyAggregator::Any));
        assert!(!aggregate_matches(&[false, false], PolicyAggregator::Any));
    }

    #[test]
    fn aggregator_empty_is_false() {
        assert!(!aggregate_matches(&[], PolicyAggregator::Any));
        assert!(!aggregate_matches(&[], PolicyAggregator::All));
    }

    #[test]
    fn string_is_and_isnot() {
        assert!(check_string("MIT", "MIT", PolicyOperator::Is));
        assert!(!check_string("MIT", "Apache-2.0", PolicyOperator::Is));
        assert!(check_string("MIT", "Apache-2.0", PolicyOperator::IsNot));
    }

    #[test]
    fn string_regex_matches() {
        assert!(check_string("^GPL-.*", "GPL-3.0", PolicyOperator::Matches));
        assert!(check_string("^GPL-.*", "MIT", PolicyOperator::NoMatch));
    }

    #[test]
    fn numeric_comparisons() {
        assert!(check_numeric(7.0, 8.0, PolicyOperator::NumericGreaterThan));
        assert!(check_numeric(7.0, 7.0, PolicyOperator::NumericEqual));
        assert!(check_numeric(7.0, 6.0, PolicyOperator::NumericLessThan));
        assert!(check_numeric(7.0, 7.0, PolicyOperator::NumericGreaterThanOrEqual));
        assert!(check_numeric(7.0, 7.0, PolicyOperator::NumericLessThanOrEqual));
        assert!(check_numeric(7.0, 8.0, PolicyOperator::NumericNotEqual));
    }

    #[test]
    fn applies_to_missing_field() {
        let mut c = Component::new(Uuid::new_v4(), "x");
        c.purl = None;
        let cond = PolicyCondition {
            subject: Subject::PackageUrl,
            operator: PolicyOperator::Is,
            value: "pkg:cargo/x".into(),
        };
        assert!(!applies_to(&cond, &c));
        c.purl = Some("pkg:cargo/x".into());
        assert!(applies_to(&cond, &c));
    }

    #[test]
    fn policy_builder_chains_conditions() {
        let p = Policy::new("strict")
            .with_condition(PolicyCondition {
                subject: Subject::License,
                operator: PolicyOperator::Is,
                value: "GPL-3.0".into(),
            })
            .with_condition(PolicyCondition {
                subject: Subject::Severity,
                operator: PolicyOperator::Is,
                value: "CRITICAL".into(),
            });
        assert_eq!(p.conditions.len(), 2);
        assert_eq!(p.violation_state, "FAIL");
    }
}
