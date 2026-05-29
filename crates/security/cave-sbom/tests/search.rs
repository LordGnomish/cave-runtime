// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD: failing tests for cross-entity keyword search (SearchResource parity).

use cave_sbom::components::{ComponentRecord, Project};
use cave_sbom::models::{AffectedRange, AnalysisState, Severity, VulnIntel, VulnSource};
use cave_sbom::search::{SearchResult, SearchResultKind, search_all};
use uuid::Uuid;

fn mk_project(name: &str) -> Project {
    Project::new(name, Some("1.0.0".into()))
}

fn mk_comp(name: &str, version: &str, license: Option<&str>) -> ComponentRecord {
    let p = Uuid::new_v4();
    let mut c = ComponentRecord::new(p, name, version);
    c.license = license.map(|s| s.into());
    c.purl = Some(format!("pkg:npm/{}@{}", name, version));
    c
}

fn mk_vuln(vuln_id: &str, title: &str) -> VulnIntel {
    VulnIntel {
        id: Uuid::new_v4(),
        vuln_id: vuln_id.into(),
        source: VulnSource::Nvd,
        title: title.into(),
        description: format!("description for {}", vuln_id),
        severity: Severity::High,
        cvss_v3_base: Some(7.5),
        cvss_v3_vector: None,
        cvss_v2_base: None,
        epss_score: None,
        epss_percentile: None,
        cwes: vec![],
        references: vec![],
        affected: vec![AffectedRange {
            purl_type: "npm".into(),
            namespace: None,
            name: "log4j".into(),
            vers: "*".into(),
            fixed: None,
        }],
        published: None,
        modified: None,
        state: AnalysisState::NotSet,
    }
}

// ─── 1. Basic keyword search ─────────────────────────────────────────────────

#[test]
fn search_finds_project_by_name() {
    let projects = vec![mk_project("my-web-app"), mk_project("other-service")];
    let results = search_all("my-web", &projects, &[], &[]);
    let kinds: Vec<_> = results.iter().map(|r| &r.kind).collect();
    assert!(
        kinds.iter().any(|k| matches!(k, SearchResultKind::Project)),
        "should find project by name substring"
    );
    assert!(
        results.iter().any(|r| r.label.contains("my-web-app")),
        "result label must contain the project name"
    );
}

#[test]
fn search_finds_component_by_name() {
    let comps = vec![
        mk_comp("log4j", "2.17.1", Some("Apache-2.0")),
        mk_comp("lodash", "4.17.21", Some("MIT")),
    ];
    let results = search_all("log4", &[], &comps, &[]);
    assert!(
        results.iter().any(|r| {
            matches!(r.kind, SearchResultKind::Component) && r.label.contains("log4j")
        }),
        "should find component named log4j via 'log4' query"
    );
}

#[test]
fn search_finds_vulnerability_by_id() {
    let vulns = vec![
        mk_vuln("CVE-2021-44228", "Log4Shell"),
        mk_vuln("CVE-2022-22965", "Spring4Shell"),
    ];
    let results = search_all("CVE-2021-44228", &[], &[], &vulns);
    assert!(
        results.iter().any(|r| {
            matches!(r.kind, SearchResultKind::Vulnerability)
                && r.label.contains("CVE-2021-44228")
        }),
        "should find vulnerability by exact CVE ID"
    );
}

#[test]
fn search_finds_vulnerability_by_title() {
    let vulns = vec![mk_vuln("CVE-2021-44228", "Log4Shell Remote Code Execution")];
    let results = search_all("Log4Shell", &[], &[], &vulns);
    assert!(
        results.iter().any(|r| matches!(r.kind, SearchResultKind::Vulnerability)),
        "should find vulnerability by title keyword"
    );
}

// ─── 2. Cross-entity results ─────────────────────────────────────────────────

#[test]
fn search_returns_results_from_all_entity_types() {
    let projects = vec![mk_project("log4j-app")];
    let comps = vec![mk_comp("log4j", "2.17.1", Some("Apache-2.0"))];
    let vulns = vec![mk_vuln("CVE-2021-44228", "Log4Shell (log4j)")];
    let results = search_all("log4j", &projects, &comps, &vulns);
    let has_proj = results.iter().any(|r| matches!(r.kind, SearchResultKind::Project));
    let has_comp = results.iter().any(|r| matches!(r.kind, SearchResultKind::Component));
    let has_vuln = results.iter().any(|r| matches!(r.kind, SearchResultKind::Vulnerability));
    assert!(has_proj, "should return project results");
    assert!(has_comp, "should return component results");
    assert!(has_vuln, "should return vulnerability results");
}

// ─── 3. Empty and no-match cases ─────────────────────────────────────────────

#[test]
fn search_empty_query_returns_nothing() {
    let projects = vec![mk_project("my-app")];
    let results = search_all("", &projects, &[], &[]);
    assert!(
        results.is_empty(),
        "empty query should return no results"
    );
}

#[test]
fn search_no_match_returns_empty() {
    let projects = vec![mk_project("backend")];
    let comps = vec![mk_comp("lodash", "4.17.21", Some("MIT"))];
    let results = search_all("xyzzy-nonexistent", &projects, &comps, &[]);
    assert!(results.is_empty(), "no-match query should return empty list");
}

// ─── 4. SearchResult structure ───────────────────────────────────────────────

#[test]
fn search_result_has_uuid_and_kind() {
    let projects = vec![mk_project("my-app")];
    let results = search_all("my-app", &projects, &[], &[]);
    assert!(!results.is_empty());
    let r = &results[0];
    // uuid must parse as valid UUID string
    assert!(!r.uuid.to_string().is_empty());
    assert!(matches!(r.kind, SearchResultKind::Project));
}

// ─── 5. Case-insensitive matching ────────────────────────────────────────────

#[test]
fn search_is_case_insensitive() {
    let comps = vec![mk_comp("OpenSSL", "3.0.0", Some("Apache-2.0"))];
    let results_lower = search_all("openssl", &[], &comps, &[]);
    let results_upper = search_all("OPENSSL", &[], &comps, &[]);
    assert!(!results_lower.is_empty(), "lowercase query should find OpenSSL");
    assert!(!results_upper.is_empty(), "uppercase query should find OpenSSL");
}

// ─── 6. Limit / pagination awareness ─────────────────────────────────────────

#[test]
fn search_results_are_bounded_at_one_hundred() {
    // Create 150 components matching the query.
    let comps: Vec<_> = (0..150)
        .map(|i| mk_comp(&format!("lib-{}", i), "1.0.0", Some("MIT")))
        .collect();
    let results = search_all("lib-", &[], &comps, &[]);
    assert!(
        results.len() <= 100,
        "search must cap results at 100 to prevent unbounded responses (got {})",
        results.len()
    );
}
