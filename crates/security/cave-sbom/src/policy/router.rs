// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/policy/PolicyEngine.java (decision routing)
//
//! Policy decision routing — once `evaluate_pipeline` returns the raw
//! violations, the router classifies them into outcome buckets so the
//! ingest/CI integration knows whether to block, warn, or just record.

use super::{PolicyViolation, ViolationState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Decision {
    /// `Fail` violations present — caller must block the build / publish.
    Block { reasons: Vec<String> },
    /// Only `Warn` violations.
    Warn { reasons: Vec<String> },
    /// Only `Info` violations or no violations at all.
    Accept { reasons: Vec<String> },
}

impl Decision {
    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::Block { .. })
    }
    pub fn reasons(&self) -> &[String] {
        match self {
            Self::Block { reasons } | Self::Warn { reasons } | Self::Accept { reasons } => reasons,
        }
    }
}

/// Drive a list of violations into a single decision. Fail beats Warn beats
/// Info — mirrors DependencyTrack's `Severity.compare` precedence at the
/// `ViolationState` level.
pub fn decide(violations: &[PolicyViolation]) -> Decision {
    let mut fail = Vec::new();
    let mut warn = Vec::new();
    let mut info = Vec::new();
    for v in violations {
        let line = format!("{} → {}", v.policy_name, v.message);
        match v.violation_state {
            ViolationState::Fail => fail.push(line),
            ViolationState::Warn => warn.push(line),
            ViolationState::Info => info.push(line),
        }
    }
    if !fail.is_empty() {
        Decision::Block { reasons: fail }
    } else if !warn.is_empty() {
        Decision::Warn { reasons: warn }
    } else {
        Decision::Accept { reasons: info }
    }
}

/// Aggregate `(policy_uuid, violation_state)` counts. Useful for portfolio
/// dashboards.
pub fn group_by_policy(
    violations: &[PolicyViolation],
) -> BTreeMap<Uuid, PolicyDecisionSummary> {
    let mut map: BTreeMap<Uuid, PolicyDecisionSummary> = BTreeMap::new();
    for v in violations {
        let entry = map.entry(v.policy_uuid).or_insert_with(|| PolicyDecisionSummary {
            policy_uuid: v.policy_uuid,
            policy_name: v.policy_name.clone(),
            fail: 0,
            warn: 0,
            info: 0,
        });
        match v.violation_state {
            ViolationState::Fail => entry.fail += 1,
            ViolationState::Warn => entry.warn += 1,
            ViolationState::Info => entry.info += 1,
        }
    }
    map
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyDecisionSummary {
    pub policy_uuid: Uuid,
    pub policy_name: String,
    pub fail: usize,
    pub warn: usize,
    pub info: usize,
}

impl PolicyDecisionSummary {
    pub fn total(&self) -> usize {
        self.fail + self.warn + self.info
    }
    pub fn highest_state(&self) -> Option<ViolationState> {
        if self.fail > 0 {
            Some(ViolationState::Fail)
        } else if self.warn > 0 {
            Some(ViolationState::Warn)
        } else if self.info > 0 {
            Some(ViolationState::Info)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkv(state: ViolationState, policy_name: &str, msg: &str) -> PolicyViolation {
        PolicyViolation {
            policy_uuid: Uuid::nil(),
            policy_name: policy_name.to_string(),
            component_uuid: Uuid::nil(),
            condition_index: 0,
            violation_state: state,
            message: msg.to_string(),
        }
    }

    #[test]
    fn empty_violations_yields_accept() {
        let d = decide(&[]);
        assert!(matches!(d, Decision::Accept { .. }));
        assert!(!d.is_blocking());
    }

    #[test]
    fn fail_takes_precedence_over_warn_and_info() {
        let v = vec![
            mkv(ViolationState::Info, "P-info", "info"),
            mkv(ViolationState::Warn, "P-warn", "warn"),
            mkv(ViolationState::Fail, "P-fail", "fail"),
        ];
        let d = decide(&v);
        match d {
            Decision::Block { reasons } => {
                assert_eq!(reasons.len(), 1);
                assert!(reasons[0].contains("fail"));
            }
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn warn_takes_precedence_over_info() {
        let v = vec![
            mkv(ViolationState::Info, "P", "info"),
            mkv(ViolationState::Warn, "P", "warn"),
        ];
        let d = decide(&v);
        assert!(matches!(d, Decision::Warn { .. }));
    }

    #[test]
    fn info_only_yields_accept() {
        let v = vec![mkv(ViolationState::Info, "P", "info")];
        let d = decide(&v);
        assert!(matches!(d, Decision::Accept { .. }));
        assert!(!d.is_blocking());
    }

    #[test]
    fn block_collects_only_fail_lines() {
        let v = vec![
            mkv(ViolationState::Fail, "P-1", "fail-1"),
            mkv(ViolationState::Warn, "P-2", "warn"),
            mkv(ViolationState::Fail, "P-3", "fail-2"),
        ];
        let d = decide(&v);
        match d {
            Decision::Block { reasons } => assert_eq!(reasons.len(), 2),
            _ => panic!("expected Block"),
        }
    }

    #[test]
    fn group_by_policy_counts_states_per_policy() {
        let p1 = Uuid::new_v4();
        let p2 = Uuid::new_v4();
        let v = vec![
            PolicyViolation {
                policy_uuid: p1,
                policy_name: "A".into(),
                component_uuid: Uuid::nil(),
                condition_index: 0,
                violation_state: ViolationState::Fail,
                message: "f".into(),
            },
            PolicyViolation {
                policy_uuid: p1,
                policy_name: "A".into(),
                component_uuid: Uuid::nil(),
                condition_index: 1,
                violation_state: ViolationState::Warn,
                message: "w".into(),
            },
            PolicyViolation {
                policy_uuid: p2,
                policy_name: "B".into(),
                component_uuid: Uuid::nil(),
                condition_index: 0,
                violation_state: ViolationState::Info,
                message: "i".into(),
            },
        ];
        let m = group_by_policy(&v);
        assert_eq!(m.len(), 2);
        assert_eq!(m[&p1].fail, 1);
        assert_eq!(m[&p1].warn, 1);
        assert_eq!(m[&p1].total(), 2);
        assert_eq!(m[&p1].highest_state(), Some(ViolationState::Fail));
        assert_eq!(m[&p2].highest_state(), Some(ViolationState::Info));
    }

    #[test]
    fn decision_reasons_accessible_via_method() {
        let v = vec![mkv(ViolationState::Fail, "P", "boom")];
        let d = decide(&v);
        assert_eq!(d.reasons().len(), 1);
    }

    #[test]
    fn summary_total_sums_states() {
        let s = PolicyDecisionSummary {
            policy_uuid: Uuid::nil(),
            policy_name: "x".into(),
            fail: 2,
            warn: 3,
            info: 4,
        };
        assert_eq!(s.total(), 9);
    }

    #[test]
    fn summary_highest_state_is_none_when_empty() {
        let s = PolicyDecisionSummary {
            policy_uuid: Uuid::nil(),
            policy_name: "x".into(),
            fail: 0,
            warn: 0,
            info: 0,
        };
        assert!(s.highest_state().is_none());
    }

    #[test]
    fn decision_serde_roundtrip() {
        let d = Decision::Block {
            reasons: vec!["x → y".into()],
        };
        let j = serde_json::to_string(&d).unwrap();
        let back: Decision = serde_json::from_str(&j).unwrap();
        assert_eq!(d, back);
    }
}
