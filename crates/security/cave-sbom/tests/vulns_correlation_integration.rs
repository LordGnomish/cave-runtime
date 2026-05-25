// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Integration: feed cave-sbom components into a vulnerability finding
//! correlator. We use a trait + in-test mock to keep cave-vulns out of the
//! deps of cave-sbom — the mock has the same shape (purl + cve + severity)
//! as a real finding ingest would produce.

use cave_sbom::components::ComponentRecord;
use cave_sbom::models::{AffectedRange, AnalysisState, Severity, VulnIntel, VulnSource};
use cave_sbom::portfolio::ProjectRisk;
use uuid::Uuid;

/// Trait shape mirroring the cave-vulns finding-correlator contract.
trait FindingCorrelator {
    fn correlate(&self, components: &[ComponentRecord], vulns: &[VulnIntel])
    -> Vec<(Uuid, String)>;
}

/// Mock implementation: matches a component to a vuln when the component's
/// purl appears inside the vuln's affected range name.
struct PurlNameCorrelator;

impl FindingCorrelator for PurlNameCorrelator {
    fn correlate(
        &self,
        components: &[ComponentRecord],
        vulns: &[VulnIntel],
    ) -> Vec<(Uuid, String)> {
        let mut out = Vec::new();
        for c in components {
            for v in vulns {
                for a in &v.affected {
                    if c.name == a.name {
                        out.push((c.uuid, v.vuln_id.clone()));
                    }
                }
            }
        }
        out
    }
}

fn comp(name: &str) -> ComponentRecord {
    let mut c = ComponentRecord::new(Uuid::new_v4(), name, "1.0.0");
    c.purl = Some(format!("pkg:npm/{}@1.0.0", name));
    c
}

fn vuln(name: &str, sev: Severity, base: f32) -> VulnIntel {
    VulnIntel {
        id: Uuid::new_v4(),
        vuln_id: format!("CVE-{}", name),
        source: VulnSource::Nvd,
        title: "test".into(),
        description: "test".into(),
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

#[test]
fn correlator_matches_component_to_vuln_by_purl_name() {
    let comps = vec![comp("openssl"), comp("lodash"), comp("safe")];
    let vulns = vec![
        vuln("openssl", Severity::Critical, 9.8),
        vuln("lodash", Severity::High, 7.5),
    ];
    let hits = PurlNameCorrelator.correlate(&comps, &vulns);
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().any(|(_, v)| v == "CVE-openssl"));
    assert!(hits.iter().any(|(_, v)| v == "CVE-lodash"));
}

#[test]
fn correlator_no_hits_on_clean_components() {
    let comps = vec![comp("safe1"), comp("safe2")];
    let vulns = vec![vuln("dangerous", Severity::High, 7.5)];
    let hits = PurlNameCorrelator.correlate(&comps, &vulns);
    assert!(hits.is_empty());
}

#[test]
fn project_risk_score_aggregates_vulnerable_components() {
    let p = Uuid::new_v4();
    let mut openssl = ComponentRecord::new(p, "openssl", "1.0.0");
    openssl.purl = Some("pkg:apk/openssl@1.0.0".into());
    let mut lodash = ComponentRecord::new(p, "lodash", "4.0.0");
    lodash.purl = Some("pkg:npm/lodash@4.0.0".into());
    let comps = vec![openssl, lodash];
    let vulns = vec![
        vuln("openssl", Severity::Critical, 9.8),
        vuln("lodash", Severity::High, 7.5),
    ];
    let risk = ProjectRisk::compute(p, &comps, &vulns);
    assert_eq!(risk.total_components, 2);
    assert_eq!(risk.vulnerable_components, 2);
    assert_eq!(risk.critical, 1);
    assert_eq!(risk.high, 1);
    // 10 (critical) + 5 (high) = 15
    assert_eq!(risk.inherited_risk_score, 15.0);
}

#[test]
fn correlator_returns_multiple_vulns_per_component() {
    let comps = vec![comp("openssl")];
    let vulns = vec![
        vuln("openssl", Severity::Critical, 9.8),
        vuln("openssl", Severity::High, 7.5),
    ];
    let hits = PurlNameCorrelator.correlate(&comps, &vulns);
    assert_eq!(hits.len(), 2);
}
