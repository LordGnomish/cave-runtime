// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/policy/VulnerabilityIdPolicyEvaluator.java
//
//! Vulnerability-ID evaluator — checks whether a specific vulnerability
//! identifier (CVE, GHSA, OSV, etc.) is present for a component.
//!
//! Mirrors DependencyTrack's `VulnerabilityIdPolicyEvaluator`.

use crate::components::ComponentRecord;
use crate::models::VulnIntel;

fn component_matches_vuln(c: &ComponentRecord, v: &VulnIntel) -> bool {
    if let Some(pu) = &c.purl {
        for a in &v.affected {
            if pu.contains(&a.name) {
                return true;
            }
        }
    }
    v.affected.iter().any(|a| a.name == c.name)
}

/// Evaluate whether a component has a specific vulnerability ID.
///
/// `vuln_id` is matched case-insensitively against `VulnIntel::vuln_id`.
/// Returns `Some(message)` when a matching vulnerability affects the component.
pub fn component_has_vuln_id(
    c: &ComponentRecord,
    vulns: &[VulnIntel],
    vuln_id: &str,
) -> Option<String> {
    let lower_target = vuln_id.to_lowercase();
    for v in vulns {
        if v.vuln_id.to_lowercase() == lower_target && component_matches_vuln(c, v) {
            return Some(format!(
                "{} is affected by blocked vulnerability {}",
                c.name, vuln_id
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AffectedRange, AnalysisState, Severity, VulnSource};
    use uuid::Uuid;

    fn mk_vuln(name: &str, id: &str) -> VulnIntel {
        VulnIntel {
            id: Uuid::new_v4(),
            vuln_id: id.into(),
            source: VulnSource::Nvd,
            title: "t".into(),
            description: "d".into(),
            severity: Severity::Critical,
            cvss_v3_base: Some(9.8),
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

    fn mk_comp(name: &str) -> ComponentRecord {
        let mut c = ComponentRecord::new(Uuid::new_v4(), name, "1.0.0");
        c.purl = Some(format!("pkg:npm/{}@1.0.0", name));
        c
    }

    #[test]
    fn exact_cve_match_fires() {
        let c = mk_comp("openssl");
        let v = mk_vuln("openssl", "CVE-2014-0160");
        assert!(component_has_vuln_id(&c, &[v], "CVE-2014-0160").is_some());
    }

    #[test]
    fn case_insensitive_match() {
        let c = mk_comp("openssl");
        let v = mk_vuln("openssl", "CVE-2014-0160");
        assert!(component_has_vuln_id(&c, &[v], "cve-2014-0160").is_some());
    }

    #[test]
    fn different_id_no_match() {
        let c = mk_comp("openssl");
        let v = mk_vuln("openssl", "CVE-2014-0160");
        assert!(component_has_vuln_id(&c, &[v], "CVE-9999-9999").is_none());
    }

    #[test]
    fn unaffected_component_no_match() {
        let c = mk_comp("other");
        let v = mk_vuln("openssl", "CVE-2014-0160");
        assert!(component_has_vuln_id(&c, &[v], "CVE-2014-0160").is_none());
    }

    #[test]
    fn ghsa_id_also_matches() {
        let c = mk_comp("lodash");
        let v = mk_vuln("lodash", "GHSA-abc-def-1234");
        assert!(component_has_vuln_id(&c, &[v], "GHSA-abc-def-1234").is_some());
    }
}
