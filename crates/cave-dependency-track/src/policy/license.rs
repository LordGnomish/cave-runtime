// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! License + license-group policy evaluator.
//! Mirrors `LicensePolicyEvaluator` + `LicenseGroupPolicyEvaluator`.

use super::engine::{
    PolicyCondition, PolicyResult, Subject, ViolationKind, applies_to, check_string,
};
use crate::models::Component;
use std::collections::HashMap;
use uuid::Uuid;

pub fn evaluate_license(
    policy: Uuid,
    conditions: &[PolicyCondition],
    license_groups: &HashMap<String, Vec<String>>,
    component: &Component,
) -> Vec<PolicyResult> {
    let mut out = Vec::new();
    for (i, c) in conditions.iter().enumerate() {
        if !applies_to(c, component) {
            continue;
        }
        let component_license = component
            .license
            .as_deref()
            .or(component.license_expression.as_deref())
            .unwrap_or("");
        let hit = match c.subject {
            Subject::License => check_string(&c.value, component_license, c.operator),
            Subject::LicenseGroup => {
                license_groups
                    .get(&c.value)
                    .map(|members| members.iter().any(|m| m == component_license))
                    .unwrap_or(false)
            }
            _ => continue,
        };
        if hit {
            out.push(PolicyResult {
                policy,
                component: component.uuid,
                kind: ViolationKind::License,
                condition_index: i,
                reason: format!(
                    "{:?} {:?} {} (component={})",
                    c.subject, c.operator, c.value, component_license
                ),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::engine::PolicyOperator;

    fn comp(license: Option<&str>) -> Component {
        let mut c = Component::new(Uuid::new_v4(), "lib");
        c.license = license.map(|s| s.into());
        c
    }

    #[test]
    fn flags_disallowed_license() {
        let cond = PolicyCondition {
            subject: Subject::License,
            operator: PolicyOperator::Is,
            value: "GPL-3.0".into(),
        };
        let v = evaluate_license(Uuid::new_v4(), &[cond], &HashMap::new(), &comp(Some("GPL-3.0")));
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn license_group_match_flags_violation() {
        let cond = PolicyCondition {
            subject: Subject::LicenseGroup,
            operator: PolicyOperator::Is,
            value: "Copyleft".into(),
        };
        let mut groups = HashMap::new();
        groups.insert(
            "Copyleft".to_string(),
            vec!["GPL-3.0".into(), "AGPL-3.0".into()],
        );
        let v = evaluate_license(Uuid::new_v4(), &[cond], &groups, &comp(Some("AGPL-3.0")));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, ViolationKind::License);
    }

    #[test]
    fn no_match_no_violation() {
        let cond = PolicyCondition {
            subject: Subject::License,
            operator: PolicyOperator::Is,
            value: "GPL-3.0".into(),
        };
        let v = evaluate_license(Uuid::new_v4(), &[cond], &HashMap::new(), &comp(Some("MIT")));
        assert!(v.is_empty());
    }

    #[test]
    fn missing_license_skipped() {
        let cond = PolicyCondition {
            subject: Subject::License,
            operator: PolicyOperator::Is,
            value: "GPL-3.0".into(),
        };
        let v = evaluate_license(Uuid::new_v4(), &[cond], &HashMap::new(), &comp(None));
        assert!(v.is_empty());
    }

    #[test]
    fn unknown_license_group_doesnt_match() {
        let cond = PolicyCondition {
            subject: Subject::LicenseGroup,
            operator: PolicyOperator::Is,
            value: "Unknown".into(),
        };
        let v = evaluate_license(
            Uuid::new_v4(),
            &[cond],
            &HashMap::new(),
            &comp(Some("MIT")),
        );
        assert!(v.is_empty());
    }
}
