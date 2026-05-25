// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/policy/SeverityPolicyEvaluator.java
//   src/main/java/org/dependencytrack/policy/CwePolicyEvaluator.java
//
//! Vulnerability policy evaluator — severity threshold + CVSS threshold.

use crate::components::ComponentRecord;
use crate::models::{Severity, VulnIntel};

fn component_matches_vuln(c: &ComponentRecord, v: &VulnIntel) -> bool {
    // Match purl exactly, or affected-range name == component name.
    if let Some(pu) = &c.purl {
        for a in &v.affected {
            if pu.contains(&a.name) {
                return true;
            }
        }
    }
    v.affected.iter().any(|a| a.name == c.name)
}

pub fn component_has_severity_at_least(
    c: &ComponentRecord,
    vulns: &[VulnIntel],
    min: Severity,
) -> Option<String> {
    for v in vulns {
        if v.severity >= min && component_matches_vuln(c, v) {
            return Some(format!(
                "{} has vulnerability {} at severity {:?}",
                c.name, v.vuln_id, v.severity
            ));
        }
    }
    None
}

pub fn component_has_cvss_at_least(
    c: &ComponentRecord,
    vulns: &[VulnIntel],
    min: f32,
) -> Option<String> {
    for v in vulns {
        if v.cvss_v3_base.unwrap_or(0.0) >= min && component_matches_vuln(c, v) {
            return Some(format!(
                "{} has vulnerability {} with CVSS {}",
                c.name,
                v.vuln_id,
                v.cvss_v3_base.unwrap_or(0.0)
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AffectedRange, AnalysisState, VulnSource};
    use uuid::Uuid;

    fn vuln(name: &str, base: f32, sev: Severity) -> VulnIntel {
        VulnIntel {
            id: Uuid::new_v4(),
            vuln_id: format!("CVE-{}", name),
            source: VulnSource::Nvd,
            title: "".into(),
            description: "".into(),
            severity: sev,
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
                name: name.into(),
                vers: "*".into(),
                fixed: None,
            }],
            published: None,
            modified: None,
            state: AnalysisState::NotSet,
        }
    }

    fn comp(name: &str) -> ComponentRecord {
        let mut c = ComponentRecord::new(Uuid::new_v4(), name, "1.0.0");
        c.purl = Some(format!("pkg:npm/{}@1.0.0", name));
        c
    }

    #[test]
    fn severity_threshold_fires_on_critical() {
        assert!(
            component_has_severity_at_least(
                &comp("openssl"),
                &[vuln("openssl", 9.8, Severity::Critical)],
                Severity::High
            )
            .is_some()
        );
    }

    #[test]
    fn severity_threshold_skips_below() {
        assert!(
            component_has_severity_at_least(
                &comp("openssl"),
                &[vuln("openssl", 5.0, Severity::Medium)],
                Severity::High
            )
            .is_none()
        );
    }

    #[test]
    fn severity_skips_non_matching_component() {
        assert!(
            component_has_severity_at_least(
                &comp("other"),
                &[vuln("openssl", 9.8, Severity::Critical)],
                Severity::High
            )
            .is_none()
        );
    }

    #[test]
    fn cvss_threshold_above_fires() {
        assert!(
            component_has_cvss_at_least(
                &comp("openssl"),
                &[vuln("openssl", 9.0, Severity::Critical)],
                7.0
            )
            .is_some()
        );
    }

    #[test]
    fn cvss_threshold_below_does_not_fire() {
        assert!(
            component_has_cvss_at_least(
                &comp("openssl"),
                &[vuln("openssl", 5.0, Severity::Medium)],
                7.0
            )
            .is_none()
        );
    }
}
