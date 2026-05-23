// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Coordinates / PURL / CPE policy evaluator.
//! Mirrors `CoordinatesPolicyEvaluator` + `PackageURLPolicyEvaluator` +
//! `CpePolicyEvaluator`.

use super::engine::{
    PolicyCondition, PolicyResult, Subject, ViolationKind, applies_to, check_string,
};
use crate::models::Component;
use uuid::Uuid;

pub fn evaluate_coordinates(
    policy: Uuid,
    conditions: &[PolicyCondition],
    component: &Component,
) -> Vec<PolicyResult> {
    let mut out = Vec::new();
    for (i, c) in conditions.iter().enumerate() {
        if !applies_to(c, component) {
            continue;
        }
        let (label, value) = match c.subject {
            Subject::PackageUrl => ("purl", component.purl.as_deref().unwrap_or("")),
            Subject::Cpe => ("cpe", component.cpe.as_deref().unwrap_or("")),
            Subject::Coordinates => (
                "coords",
                component.purl.as_deref().or(component.cpe.as_deref()).unwrap_or(""),
            ),
            Subject::Version => ("version", component.version.as_deref().unwrap_or("")),
            Subject::ComponentHash => (
                "sha256",
                component.sha256.as_deref().or(component.sha1.as_deref()).unwrap_or(""),
            ),
            _ => continue,
        };
        if check_string(&c.value, value, c.operator) {
            out.push(PolicyResult {
                policy,
                component: component.uuid,
                kind: ViolationKind::Operational,
                condition_index: i,
                reason: format!("{} {:?} {}: component={}", label, c.operator, c.value, value),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::engine::PolicyOperator;

    fn comp(purl: &str, cpe: Option<&str>, sha256: Option<&str>) -> Component {
        let mut c = Component::new(Uuid::new_v4(), "lib");
        c.purl = Some(purl.into());
        c.cpe = cpe.map(|s| s.into());
        c.sha256 = sha256.map(|s| s.into());
        c
    }

    #[test]
    fn purl_match_flags_violation() {
        let cond = PolicyCondition {
            subject: Subject::PackageUrl,
            operator: PolicyOperator::Matches,
            value: "^pkg:npm/event-stream@".into(),
        };
        let v = evaluate_coordinates(
            Uuid::new_v4(),
            &[cond],
            &comp("pkg:npm/event-stream@3.3.6", None, None),
        );
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn cpe_exact_match() {
        let cond = PolicyCondition {
            subject: Subject::Cpe,
            operator: PolicyOperator::Is,
            value: "cpe:2.3:a:openssl:openssl:3.0.0:*:*:*:*:*:*:*".into(),
        };
        let v = evaluate_coordinates(
            Uuid::new_v4(),
            &[cond],
            &comp("pkg:x/y@1", Some("cpe:2.3:a:openssl:openssl:3.0.0:*:*:*:*:*:*:*"), None),
        );
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn coordinates_falls_back_cpe_when_no_purl() {
        let mut c = comp("", Some("cpe:..."), None);
        c.purl = None;
        let cond = PolicyCondition {
            subject: Subject::Coordinates,
            operator: PolicyOperator::Is,
            value: "cpe:...".into(),
        };
        let v = evaluate_coordinates(Uuid::new_v4(), &[cond], &c);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn no_match_no_violation() {
        let cond = PolicyCondition {
            subject: Subject::PackageUrl,
            operator: PolicyOperator::Is,
            value: "pkg:npm/safe".into(),
        };
        let v = evaluate_coordinates(
            Uuid::new_v4(),
            &[cond],
            &comp("pkg:cargo/serde@1", None, None),
        );
        assert!(v.is_empty());
    }

    #[test]
    fn hash_match() {
        let cond = PolicyCondition {
            subject: Subject::ComponentHash,
            operator: PolicyOperator::Is,
            value: "deadbeef".into(),
        };
        let v = evaluate_coordinates(
            Uuid::new_v4(),
            &[cond],
            &comp("pkg:x/y@1", None, Some("deadbeef")),
        );
        assert_eq!(v.len(), 1);
    }
}
