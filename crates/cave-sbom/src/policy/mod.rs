// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/policy/PolicyEngine.java
//   src/main/java/org/dependencytrack/policy/{LicensePolicyEvaluator,SeverityPolicyEvaluator,ComponentAgePolicyEvaluator,CoordinatesPolicyEvaluator}.java
//
//! Policy engine — license / vulnerability / age / coordinates evaluators.

pub mod age;
pub mod coordinates;
pub mod license;
pub mod vuln;

use crate::components::ComponentRecord;
use crate::models::VulnIntel;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Mirror of `org.dependencytrack.model.Policy`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Policy {
    pub uuid: Uuid,
    pub name: String,
    pub violation_state: ViolationState,
    pub operator: Operator,
    pub conditions: Vec<PolicyCondition>,
}

/// Mirror of `org.dependencytrack.model.Policy.ViolationState`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ViolationState {
    Info,
    Warn,
    Fail,
}

/// Mirror of `org.dependencytrack.model.Policy.Operator` — ALL/ANY.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Operator {
    Any,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PolicyCondition {
    LicenseAllow { allow: Vec<String> },
    LicenseDeny { deny: Vec<String> },
    SeverityAtLeast { min_severity: crate::models::Severity },
    CvssAtLeast { min_cvss_v3: f32 },
    AgeOlderThanDays { days: u32 },
    CoordinatesMatch { group: Option<String>, name: String, version: Option<String> },
}

/// One concrete violation, mirroring `org.dependencytrack.model.PolicyViolation`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyViolation {
    pub policy_uuid: Uuid,
    pub policy_name: String,
    pub component_uuid: Uuid,
    pub condition_index: usize,
    pub violation_state: ViolationState,
    pub message: String,
}

/// Evaluator pipeline: evaluate every policy against every component, returning
/// the list of violations. Mirrors `PolicyEngine.applyPolicies(Set<UUID>)`.
pub fn evaluate_pipeline(
    policies: &[Policy],
    components: &[ComponentRecord],
    vulns: &[VulnIntel],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<PolicyViolation> {
    let mut out = Vec::new();
    for p in policies {
        for c in components {
            // For each component, evaluate each condition.
            let mut hits: Vec<(usize, String)> = Vec::new();
            for (i, cond) in p.conditions.iter().enumerate() {
                let hit = match cond {
                    PolicyCondition::LicenseAllow { allow } => license::violates_allow(c, allow),
                    PolicyCondition::LicenseDeny { deny } => license::violates_deny(c, deny),
                    PolicyCondition::SeverityAtLeast { min_severity } => {
                        vuln::component_has_severity_at_least(c, vulns, *min_severity)
                    }
                    PolicyCondition::CvssAtLeast { min_cvss_v3 } => {
                        vuln::component_has_cvss_at_least(c, vulns, *min_cvss_v3)
                    }
                    PolicyCondition::AgeOlderThanDays { days } => {
                        age::violates(c, *days, now)
                    }
                    PolicyCondition::CoordinatesMatch { group, name, version } => {
                        coordinates::violates(c, group.as_deref(), name, version.as_deref())
                    }
                };
                if let Some(msg) = hit {
                    hits.push((i, msg));
                }
            }
            let trigger = match p.operator {
                Operator::Any => !hits.is_empty(),
                Operator::All => hits.len() == p.conditions.len() && !hits.is_empty(),
            };
            if trigger {
                for (i, m) in hits {
                    out.push(PolicyViolation {
                        policy_uuid: p.uuid,
                        policy_name: p.name.clone(),
                        component_uuid: c.uuid,
                        condition_index: i,
                        violation_state: p.violation_state,
                        message: m,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ComponentRecord;
    use crate::models::{AffectedRange, AnalysisState, Severity, VulnIntel, VulnSource};
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn mk_policy(conds: Vec<PolicyCondition>, op: Operator, st: ViolationState) -> Policy {
        Policy {
            uuid: Uuid::new_v4(),
            name: "P".into(),
            violation_state: st,
            operator: op,
            conditions: conds,
        }
    }

    fn mk_component(name: &str, license: Option<&str>) -> ComponentRecord {
        let pu = Uuid::new_v4();
        let mut c = ComponentRecord::new(pu, name, "1.0.0");
        c.license = license.map(|s| s.into());
        c.purl = Some(format!("pkg:npm/{}@1.0.0", name));
        c
    }

    fn mk_vuln(name: &str, base: f32) -> VulnIntel {
        VulnIntel {
            id: Uuid::new_v4(),
            vuln_id: format!("CVE-{}", name),
            source: VulnSource::Nvd,
            title: "t".into(),
            description: "d".into(),
            severity: Severity::from_cvss_v3(base),
            cvss_v3_base: Some(base),
            cvss_v3_vector: None,
            cvss_v2_base: None,
            epss_score: None,
            epss_percentile: None,
            cwes: vec![],
            references: vec![],
            affected: vec![AffectedRange {
                purl_type: "npm".into(),
                namespace: None,
                name: name.to_string(),
                vers: "*".into(),
                fixed: None,
            }],
            published: None,
            modified: None,
            state: AnalysisState::NotSet,
        }
    }

    #[test]
    fn license_deny_triggers_violation() {
        let p = mk_policy(
            vec![PolicyCondition::LicenseDeny { deny: vec!["GPL-3.0".into()] }],
            Operator::Any,
            ViolationState::Fail,
        );
        let c = mk_component("lodash", Some("GPL-3.0"));
        let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
        assert_eq!(viols.len(), 1);
        assert_eq!(viols[0].violation_state, ViolationState::Fail);
    }

    #[test]
    fn license_allow_violates_when_not_in_list() {
        let p = mk_policy(
            vec![PolicyCondition::LicenseAllow { allow: vec!["MIT".into(), "Apache-2.0".into()] }],
            Operator::Any,
            ViolationState::Warn,
        );
        let bad = mk_component("foo", Some("GPL-3.0"));
        let ok = mk_component("bar", Some("MIT"));
        let viols = evaluate_pipeline(&[p], &[bad, ok], &[], Utc::now());
        assert_eq!(viols.len(), 1);
    }

    #[test]
    fn severity_threshold_finds_critical_vuln() {
        let p = mk_policy(
            vec![PolicyCondition::SeverityAtLeast { min_severity: Severity::High }],
            Operator::Any,
            ViolationState::Fail,
        );
        let c = mk_component("openssl", Some("Apache-2.0"));
        let v = mk_vuln("openssl", 9.8);
        let viols = evaluate_pipeline(&[p], &[c], &[v], Utc::now());
        assert_eq!(viols.len(), 1);
    }

    #[test]
    fn cvss_threshold_below_does_not_fire() {
        let p = mk_policy(
            vec![PolicyCondition::CvssAtLeast { min_cvss_v3: 9.0 }],
            Operator::Any,
            ViolationState::Fail,
        );
        let c = mk_component("openssl", Some("Apache-2.0"));
        let v = mk_vuln("openssl", 7.4);
        let viols = evaluate_pipeline(&[p], &[c], &[v], Utc::now());
        assert!(viols.is_empty());
    }

    #[test]
    fn age_policy_flags_old_component() {
        let p = mk_policy(
            vec![PolicyCondition::AgeOlderThanDays { days: 365 }],
            Operator::Any,
            ViolationState::Warn,
        );
        let mut c = mk_component("legacy", Some("MIT"));
        c.published_at = Some(Utc::now() - Duration::days(800));
        let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
        assert_eq!(viols.len(), 1);
    }

    #[test]
    fn operator_all_requires_every_condition() {
        let p = mk_policy(
            vec![
                PolicyCondition::LicenseDeny { deny: vec!["GPL-3.0".into()] },
                PolicyCondition::CvssAtLeast { min_cvss_v3: 7.0 },
            ],
            Operator::All,
            ViolationState::Fail,
        );
        // Only matches license: ALL fails.
        let c = mk_component("foo", Some("GPL-3.0"));
        let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
        assert!(viols.is_empty());
    }

    #[test]
    fn operator_any_fires_on_first_match() {
        let p = mk_policy(
            vec![
                PolicyCondition::LicenseAllow { allow: vec!["MIT".into()] },
                PolicyCondition::AgeOlderThanDays { days: 999 },
            ],
            Operator::Any,
            ViolationState::Info,
        );
        let c = mk_component("foo", Some("GPL-3.0"));
        let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
        assert_eq!(viols.len(), 1);
    }

    #[test]
    fn coordinates_policy_matches_by_name() {
        let p = mk_policy(
            vec![PolicyCondition::CoordinatesMatch {
                group: None,
                name: "lodash".into(),
                version: None,
            }],
            Operator::Any,
            ViolationState::Fail,
        );
        let c = mk_component("lodash", Some("MIT"));
        let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
        assert_eq!(viols.len(), 1);
    }
}
