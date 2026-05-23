// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! High-level orchestration glue.
//!
//! Mirrors `org.dependencytrack.tasks.PolicyEvaluationTask` +
//! `org.dependencytrack.search.SearchManager`.

use crate::components::ComponentIdentity;
use crate::models::{Component, Project, Vulnerability};
use crate::policy::engine::{Policy, PolicyResult, aggregate_matches};
use crate::policy::{evaluate_coordinates, evaluate_license, evaluate_vulnerability};
use crate::risk::{RiskWeights, inherited_risk};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EvaluationSummary {
    pub project: Uuid,
    pub violations: Vec<PolicyResult>,
    pub inherited_risk: f64,
    pub components_evaluated: usize,
    pub vulnerabilities: usize,
}

pub fn evaluate_project(
    project: &Project,
    components: &[Component],
    vulns_by_component: &HashMap<Uuid, Vec<Vulnerability>>,
    policies: &[Policy],
    license_groups: &HashMap<String, Vec<String>>,
) -> EvaluationSummary {
    let mut summary = EvaluationSummary {
        project: project.uuid,
        components_evaluated: components.len(),
        ..Default::default()
    };
    for c in components {
        let empty_vec: Vec<Vulnerability> = Vec::new();
        let vulns = vulns_by_component.get(&c.uuid).unwrap_or(&empty_vec);
        summary.vulnerabilities += vulns.len();
        summary.inherited_risk += inherited_risk(vulns, RiskWeights::default());
        for policy in policies {
            let mut hits = Vec::new();
            hits.extend(evaluate_license(
                policy.uuid,
                &policy.conditions,
                license_groups,
                c,
            ));
            hits.extend(evaluate_coordinates(policy.uuid, &policy.conditions, c));
            hits.extend(evaluate_vulnerability(
                policy.uuid,
                &policy.conditions,
                c.uuid,
                vulns,
            ));
            let matches: Vec<bool> = policy
                .conditions
                .iter()
                .enumerate()
                .map(|(i, _)| hits.iter().any(|h| h.condition_index == i))
                .collect();
            if aggregate_matches(&matches, policy.aggregator) {
                summary.violations.extend(hits);
            }
        }
    }
    summary
}

/// Search a portfolio for components matching `query` against name / purl / cpe.
pub fn search_components<'a>(query: &str, components: &'a [Component]) -> Vec<&'a Component> {
    let q = query.to_ascii_lowercase();
    components
        .iter()
        .filter(|c| {
            c.name.to_ascii_lowercase().contains(&q)
                || c.purl.as_deref().map(|s| s.to_ascii_lowercase().contains(&q)).unwrap_or(false)
                || c.cpe.as_deref().map(|s| s.to_ascii_lowercase().contains(&q)).unwrap_or(false)
        })
        .collect()
}

/// Number of unique `ComponentIdentity` cache keys — pre-flight check for the
/// analyzer queue (avoid re-fetching duplicate components).
pub fn unique_identity_count(components: &[Component]) -> usize {
    components
        .iter()
        .map(|c| ComponentIdentity::of(c).cache_key())
        .collect::<HashSet<String>>()
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Classifier, Severity, VulnSource};
    use crate::policy::engine::{PolicyAggregator, PolicyCondition, PolicyOperator, Subject};

    fn vuln(id: &str, s: Severity) -> Vulnerability {
        let mut v = Vulnerability::new(id, VulnSource::Nvd);
        v.severity = s;
        v
    }

    fn comp(license: Option<&str>) -> Component {
        let mut c = Component::new(Uuid::new_v4(), "lib");
        c.license = license.map(|s| s.into());
        c.purl = Some(format!("pkg:cargo/lib@{}", uuid::Uuid::new_v4()));
        c
    }

    #[test]
    fn evaluate_aggregates_violations_under_any() {
        let p = Project::new("cave", Classifier::Application);
        let c1 = comp(Some("GPL-3.0"));
        let c2 = comp(Some("MIT"));
        let policy = Policy {
            uuid: Uuid::new_v4(),
            name: "p".into(),
            aggregator: PolicyAggregator::Any,
            conditions: vec![PolicyCondition {
                subject: Subject::License,
                operator: PolicyOperator::Is,
                value: "GPL-3.0".into(),
            }],
            violation_state: "FAIL".into(),
        };
        let s = evaluate_project(
            &p,
            &[c1, c2],
            &HashMap::new(),
            &[policy],
            &HashMap::new(),
        );
        assert_eq!(s.violations.len(), 1);
        assert_eq!(s.components_evaluated, 2);
    }

    #[test]
    fn inherited_risk_accumulates() {
        let p = Project::new("cave", Classifier::Application);
        let c = comp(None);
        let mut vbc = HashMap::new();
        vbc.insert(
            c.uuid,
            vec![vuln("CVE-1", Severity::High), vuln("CVE-2", Severity::Low)],
        );
        let s = evaluate_project(&p, &[c], &vbc, &[], &HashMap::new());
        assert!((s.inherited_risk - 6.0).abs() < f64::EPSILON);
        assert_eq!(s.vulnerabilities, 2);
    }

    #[test]
    fn search_matches_by_name_purl_or_cpe() {
        let mut c1 = Component::new(Uuid::new_v4(), "openssl");
        c1.purl = Some("pkg:generic/openssl@3".into());
        let mut c2 = Component::new(Uuid::new_v4(), "alpine");
        c2.cpe = Some("cpe:2.3:o:alpine:linux:3".into());
        let comps = [c1.clone(), c2.clone()];
        let r = search_components("ssl", &comps);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "openssl");
        let r = search_components("ALPINE", &comps);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn unique_identity_count_dedupes_by_purl() {
        let mut a = Component::new(Uuid::new_v4(), "lib");
        a.purl = Some("pkg:cargo/lib@1".into());
        let mut b = Component::new(Uuid::new_v4(), "lib");
        b.purl = Some("pkg:cargo/lib@1".into());
        let mut c = Component::new(Uuid::new_v4(), "lib");
        c.purl = Some("pkg:cargo/lib@2".into());
        assert_eq!(unique_identity_count(&[a, b, c]), 2);
    }

    #[test]
    fn empty_components_zero_summary() {
        let p = Project::new("x", Classifier::Application);
        let s = evaluate_project(&p, &[], &HashMap::new(), &[], &HashMap::new());
        assert_eq!(s.vulnerabilities, 0);
        assert_eq!(s.inherited_risk, 0.0);
    }
}
