// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test (2026-05-30) for the OpenVEX statement-evaluation
//! matcher — a faithful in-memory line-port of aquasecurity/trivy
//! `pkg/vex/openvex.go` (`OpenVEX.NotAffected` / `OpenVEX.Matches` /
//! `findingStatus`) plus the `filterVulnerabilities` flow in
//! `pkg/vex/vex.go`. The matcher suppresses this scanner's own findings
//! whose vulnerability id carries a `not_affected` or `fixed` VEX
//! statement for the matching product PURL, taking the *latest* statement
//! per (vuln, product). This is pure result-filtering logic, distinct from
//! the VEX-document *signing / attestation* path which stays in cave-sign.
//!
//! References the not-yet-existing `cave_container_scan::vex` module.

use cave_container_scan::models::{Finding, FindingCategory, Severity};
use cave_container_scan::vex::{
    finding_status, FindingStatus, OpenVex, VexJustification, VexStatement, VexStatus,
};

fn vuln_finding(cve: &str) -> Finding {
    let mut f = Finding::new(
        "VULN".to_string(),
        "Detected vulnerability".to_string(),
        FindingCategory::KnownVulnerability,
        Severity::High,
        format!("{} present", cve),
        "installed package is vulnerable".to_string(),
    );
    f.cves = vec![cve.to_string()];
    f.location.package = Some("pkg:deb/debian/libfoo@1.0".to_string());
    f
}

// ── status mapping (openvex.go findingStatus) ───────────────────────────────

#[test]
fn test_finding_status_mapping() {
    assert_eq!(finding_status(VexStatus::NotAffected), FindingStatus::NotAffected);
    assert_eq!(finding_status(VexStatus::Fixed), FindingStatus::Fixed);
    assert_eq!(
        finding_status(VexStatus::UnderInvestigation),
        FindingStatus::UnderInvestigation
    );
    assert_eq!(finding_status(VexStatus::Affected), FindingStatus::Unknown);
}

// ── Matches: product PURL + vuln id selection ───────────────────────────────

#[test]
fn test_matches_returns_statements_for_product_and_vuln() {
    let vex = OpenVex::new(
        vec![VexStatement {
            vuln_id: "CVE-2021-44228".to_string(),
            product_purl: "pkg:deb/debian/libfoo@1.0".to_string(),
            sub_components: vec![],
            status: VexStatus::NotAffected,
            justification: VexJustification::VulnerableCodeNotInExecutePath,
        }],
        "test-vex.json".to_string(),
    );

    let m = vex.matches("CVE-2021-44228", "pkg:deb/debian/libfoo@1.0", &[]);
    assert_eq!(m.len(), 1);

    // Non-matching product PURL -> no match.
    let m2 = vex.matches("CVE-2021-44228", "pkg:deb/debian/other@2.0", &[]);
    assert!(m2.is_empty());

    // Non-matching vuln id -> no match.
    let m3 = vex.matches("CVE-0000-0000", "pkg:deb/debian/libfoo@1.0", &[]);
    assert!(m3.is_empty());
}

// ── NotAffected: latest-statement-wins + status gate ────────────────────────

#[test]
fn test_not_affected_takes_latest_statement() {
    // Two statements for same (vuln, product); later one (affected) overrides.
    let vex = OpenVex::new(
        vec![
            VexStatement {
                vuln_id: "CVE-1".to_string(),
                product_purl: "pkg:deb/debian/libfoo@1.0".to_string(),
                sub_components: vec![],
                status: VexStatus::NotAffected,
                justification: VexJustification::ComponentNotPresent,
            },
            VexStatement {
                vuln_id: "CVE-1".to_string(),
                product_purl: "pkg:deb/debian/libfoo@1.0".to_string(),
                sub_components: vec![],
                status: VexStatus::Affected,
                justification: VexJustification::None,
            },
        ],
        "src".to_string(),
    );

    // latest is Affected -> NOT suppressed.
    let res = vex.not_affected("CVE-1", "pkg:deb/debian/libfoo@1.0", &[]);
    assert!(res.is_none());
}

#[test]
fn test_not_affected_suppresses_on_not_affected_and_fixed() {
    let vex = OpenVex::new(
        vec![
            VexStatement {
                vuln_id: "CVE-NA".to_string(),
                product_purl: "pkg:deb/debian/libfoo@1.0".to_string(),
                sub_components: vec![],
                status: VexStatus::NotAffected,
                justification: VexJustification::VulnerableCodeNotInExecutePath,
            },
            VexStatement {
                vuln_id: "CVE-FX".to_string(),
                product_purl: "pkg:deb/debian/libfoo@1.0".to_string(),
                sub_components: vec![],
                status: VexStatus::Fixed,
                justification: VexJustification::None,
            },
            VexStatement {
                vuln_id: "CVE-UI".to_string(),
                product_purl: "pkg:deb/debian/libfoo@1.0".to_string(),
                sub_components: vec![],
                status: VexStatus::UnderInvestigation,
                justification: VexJustification::None,
            },
        ],
        "src".to_string(),
    );

    let na = vex
        .not_affected("CVE-NA", "pkg:deb/debian/libfoo@1.0", &[])
        .expect("not_affected must suppress");
    assert_eq!(na.status, FindingStatus::NotAffected);
    assert_eq!(na.source, "src");

    let fx = vex
        .not_affected("CVE-FX", "pkg:deb/debian/libfoo@1.0", &[])
        .expect("fixed must suppress");
    assert_eq!(fx.status, FindingStatus::Fixed);

    // under_investigation does NOT suppress.
    assert!(vex
        .not_affected("CVE-UI", "pkg:deb/debian/libfoo@1.0", &[])
        .is_none());
}

// ── filter: end-to-end suppression of scanner findings ──────────────────────

#[test]
fn test_filter_removes_suppressed_findings() {
    let vex = OpenVex::new(
        vec![VexStatement {
            vuln_id: "CVE-SUPPRESS".to_string(),
            product_purl: "pkg:deb/debian/libfoo@1.0".to_string(),
            sub_components: vec![],
            status: VexStatus::NotAffected,
            justification: VexJustification::VulnerableCodeNotPresent,
        }],
        "src".to_string(),
    );

    let findings = vec![vuln_finding("CVE-SUPPRESS"), vuln_finding("CVE-KEEP")];

    let outcome = vex.filter(findings);
    // One kept (CVE-KEEP), one moved to modified (CVE-SUPPRESS).
    assert_eq!(outcome.kept.len(), 1);
    assert_eq!(outcome.kept[0].cves, vec!["CVE-KEEP".to_string()]);
    assert_eq!(outcome.modified.len(), 1);
    assert_eq!(outcome.modified[0].status, FindingStatus::NotAffected);
}

#[test]
fn test_filter_keeps_everything_when_no_statements() {
    let vex = OpenVex::new(vec![], "empty".to_string());
    let findings = vec![vuln_finding("CVE-A"), vuln_finding("CVE-B")];
    let outcome = vex.filter(findings);
    assert_eq!(outcome.kept.len(), 2);
    assert!(outcome.modified.is_empty());
}
