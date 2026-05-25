// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gap-close: public-API edge cases for cave-sbom.
//!
//! Focuses on failure modes, boundary values, state transitions, and serde
//! round-trip integrity across the SBOM/vuln-intel/policy/portfolio surface.
//! All assertions go through `pub` API only — no in-crate test-helper reuse.

use cave_sbom::components::{
    ComponentIdentity, ComponentRecord, Project, projects_for_component, version_compare,
    version_index,
};
use cave_sbom::engine::{
    build_dependency_tree, count_by_type, find_by_license, find_transitive_deps, parse_purl,
};
use cave_sbom::models::{
    AffectedRange, AnalysisState, Component, ComponentType, Sbom, SbomFormat, Severity, VulnIntel,
    VulnSource,
};
use cave_sbom::notifications::{
    Notification, NotificationGroup, NotificationLevel, NotificationRule, PublisherKind,
    jira::jira_issue_payload,
    mail::{build_subject, build_text_body},
    rule_matches,
    webhook::{slack_payload, teams_payload, webhook_payload},
};
use cave_sbom::policy::{
    Operator, Policy, PolicyCondition, ViolationState, age, coordinates, evaluate_pipeline,
    license, vuln,
};
use cave_sbom::portfolio::{PortfolioSnapshot, ProjectRisk, vulnerable_trend};
use cave_sbom::sbom::{
    BomFormat, cyclonedx,
    cyclonedx::CycloneDxError,
    detect_format, spdx,
    spdx::SpdxError,
};
use cave_sbom::vuln_intel::{
    epss::{self, EpssError, EpssScore, build_index, join_in_place},
    ghsa::{self, GhsaError},
    merge_advisories,
    nvd::{self, parse_cpe_vendor_product},
    osv, snyk,
};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

// ─── helpers ────────────────────────────────────────────────────────────────

fn mk_comp(name: &str) -> ComponentRecord {
    let mut c = ComponentRecord::new(Uuid::new_v4(), name, "1.0.0");
    c.purl = Some(format!("pkg:npm/{}@1.0.0", name));
    c
}

fn mk_vuln(name: &str, sev: Severity, cvss: Option<f32>) -> VulnIntel {
    VulnIntel {
        id: Uuid::new_v4(),
        vuln_id: format!("CVE-{}", name),
        source: VulnSource::Nvd,
        title: "".into(),
        description: "".into(),
        severity: sev,
        cvss_v3_base: cvss,
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

// ─── 1. CycloneDX parser failure modes ──────────────────────────────────────

#[test]
fn cyclonedx_json_rejects_malformed_json() {
    let bad = b"{ not json";
    let err = cyclonedx::parse_json(bad).unwrap_err();
    assert!(matches!(err, CycloneDxError::Json(_)));
}

#[test]
fn cyclonedx_json_accepts_unset_bom_format() {
    // Per upstream Validator, bomFormat is optional in the wrapper.
    let blob = br#"{"specVersion":"1.6","components":[{"type":"library","name":"x","version":"1"}]}"#;
    let r = cyclonedx::parse_json(blob).unwrap();
    assert_eq!(r.components.len(), 1);
    assert_eq!(r.components[0].name, "x");
}

#[test]
fn cyclonedx_json_unknown_component_type_defaults_to_library() {
    let blob = br#"{"bomFormat":"CycloneDX","specVersion":"1.5","components":[
        {"type":"machine-learning-model","name":"m","version":"1"}]}"#;
    let r = cyclonedx::parse_json(blob).unwrap();
    assert_eq!(r.components[0].component_type, ComponentType::Library);
}

#[test]
fn cyclonedx_json_operating_system_underscore_variant_normalises() {
    // Spec uses `operating-system`; defensive accept of underscore variant.
    let blob = br#"{"bomFormat":"CycloneDX","specVersion":"1.5","components":[
        {"type":"operating_system","name":"linux","version":"6.0"}]}"#;
    let r = cyclonedx::parse_json(blob).unwrap();
    assert_eq!(
        r.components[0].component_type,
        ComponentType::OperatingSystem
    );
}

#[test]
fn cyclonedx_json_license_expression_wins_when_no_id() {
    let blob = br#"{"bomFormat":"CycloneDX","specVersion":"1.5","components":[
        {"type":"library","name":"x","version":"1",
         "licenses":[{"expression":"(MIT OR Apache-2.0) AND BSD-3-Clause"}]}]}"#;
    let r = cyclonedx::parse_json(blob).unwrap();
    assert_eq!(
        r.components[0].license.as_deref(),
        Some("(MIT OR Apache-2.0) AND BSD-3-Clause")
    );
}

#[test]
fn cyclonedx_json_case_insensitive_format_check() {
    // Validator should accept "cyclonedx" case-insensitively per upstream.
    let blob = br#"{"bomFormat":"cyclonedx","specVersion":"1.5","components":[]}"#;
    let r = cyclonedx::parse_json(blob).unwrap();
    assert_eq!(r.format_detected, BomFormat::CycloneDxJson);
}

#[test]
fn cyclonedx_xml_handles_no_components_block() {
    let blob = br#"<?xml version="1.0"?><bom xmlns="http://cyclonedx.org/schema/bom/1.5" version="1.5"/>"#;
    let r = cyclonedx::parse_xml(blob).unwrap();
    assert!(r.components.is_empty());
    assert!(r.dependencies.is_empty());
}

// ─── 2. SPDX parser failure modes ───────────────────────────────────────────

#[test]
fn spdx_json_rejects_malformed_json() {
    let err = spdx::parse_json(b"not-json").unwrap_err();
    assert!(matches!(err, SpdxError::Json(_)));
}

#[test]
fn spdx_json_dependency_of_inverts_edge_direction() {
    // DEPENDENCY_OF: subject depends-of target → edge target→subject.
    let blob = br#"{
      "spdxVersion":"SPDX-2.3","SPDXID":"SPDXRef-DOCUMENT","name":"x",
      "packages":[
        {"SPDXID":"SPDXRef-A","name":"a","versionInfo":"1"},
        {"SPDXID":"SPDXRef-B","name":"b","versionInfo":"1"}],
      "relationships":[
        {"spdxElementId":"SPDXRef-A","relatedSpdxElement":"SPDXRef-B",
         "relationshipType":"DEPENDENCY_OF"}]
    }"#;
    let r = spdx::parse_json(blob).unwrap();
    // A is DEPENDENCY_OF B → B depends on A.
    let (parent, children) = &r.dependencies[0];
    assert_eq!(parent, "SPDXRef-B");
    assert!(children.contains(&"SPDXRef-A".to_string()));
}

#[test]
fn spdx_tag_value_skips_comments_and_blank_lines() {
    let tv = "\
# header comment
SPDXVersion: SPDX-2.3
# blank below

DocumentName: my
PackageName: p1
SPDXID: SPDXRef-p1
PackageVersion: 9
";
    let r = spdx::parse_tag_value(tv.as_bytes()).unwrap();
    assert_eq!(r.project_name.as_deref(), Some("my"));
    assert_eq!(r.components.len(), 1);
    assert_eq!(r.components[0].name, "p1");
    assert_eq!(r.components[0].version, "9");
}

#[test]
fn spdx_tag_value_rejects_non_utf8() {
    let bytes: &[u8] = &[0xff, 0xfe, 0xfa];
    let err = spdx::parse_tag_value(bytes).unwrap_err();
    assert!(matches!(err, SpdxError::TagValue(_)));
}

// ─── 3. Format detection edges ──────────────────────────────────────────────

#[test]
fn detect_format_rejects_only_whitespace() {
    assert!(detect_format(b"   \t\n\r ").is_none());
}

#[test]
fn detect_format_xml_without_bom_is_unknown() {
    // <foo> alone is not enough.
    assert_eq!(detect_format(b"<root></root>"), None);
}

#[test]
fn detect_format_spdx_json_distinct_from_cyclonedx() {
    let bom = br#"{"SPDXID":"SPDXRef-DOCUMENT","spdxVersion":"SPDX-2.3"}"#;
    assert_eq!(detect_format(bom), Some(BomFormat::SpdxJson));
}

// ─── 4. PURL canonicalisation ───────────────────────────────────────────────

#[test]
fn parse_purl_no_version_yields_none_version() {
    let p = parse_purl("pkg:cargo/serde").unwrap();
    assert_eq!(p.package_type, "cargo");
    assert_eq!(p.name, "serde");
    assert!(p.version.is_none());
}

#[test]
fn parse_purl_versionless_with_namespace() {
    let p = parse_purl("pkg:maven/org.apache/commons-lang3").unwrap();
    assert_eq!(p.namespace.as_deref(), Some("org.apache"));
    assert_eq!(p.name, "commons-lang3");
    assert!(p.version.is_none());
}

#[test]
fn parse_purl_missing_pkg_prefix_is_none() {
    assert!(parse_purl("npm/lodash@1").is_none());
}

#[test]
fn parse_purl_no_slash_after_type_is_none() {
    assert!(parse_purl("pkg:notype").is_none());
}

// ─── 5. CPE 2.3 parsing edges ───────────────────────────────────────────────

#[test]
fn parse_cpe_empty_vendor_becomes_none() {
    let (v, p) = parse_cpe_vendor_product("cpe:2.3:a::log4j:*:*:*:*:*:*:*:*");
    assert!(v.is_none());
    assert_eq!(p, "log4j");
}

#[test]
fn parse_cpe_malformed_falls_back_to_raw() {
    let (v, p) = parse_cpe_vendor_product("not-a-cpe");
    assert!(v.is_none());
    assert_eq!(p, "not-a-cpe");
}

// ─── 6. Severity boundary semantics ─────────────────────────────────────────

#[test]
fn severity_negative_cvss_clamps_to_info() {
    // Defensive: negative scores should be normalised to Info (below all thresholds).
    assert_eq!(Severity::from_cvss_v3(-1.0), Severity::Info);
}

#[test]
fn severity_serde_round_trip_all_variants() {
    for s in [
        Severity::Unassigned,
        Severity::Info,
        Severity::Low,
        Severity::Medium,
        Severity::High,
        Severity::Critical,
    ] {
        let j = serde_json::to_string(&s).unwrap();
        let back: Severity = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}

// ─── 7. Vulnerability matching ──────────────────────────────────────────────

#[test]
fn vuln_match_uses_purl_substring_when_present() {
    // The matcher checks purl.contains(affected.name). Component without
    // matching purl but matching name should still hit via the name fallback.
    let mut c = ComponentRecord::new(Uuid::new_v4(), "lodash", "1.0");
    c.purl = None; // forces name fallback
    let v = mk_vuln("lodash", Severity::High, Some(7.5));
    assert!(
        vuln::component_has_severity_at_least(&c, &[v], Severity::High).is_some()
    );
}

#[test]
fn vuln_threshold_exactly_at_boundary_fires() {
    let c = mk_comp("openssl");
    let v = mk_vuln("openssl", Severity::High, Some(7.0));
    assert!(vuln::component_has_cvss_at_least(&c, &[v], 7.0).is_some());
}

#[test]
fn vuln_threshold_just_below_boundary_does_not_fire() {
    let c = mk_comp("openssl");
    let v = mk_vuln("openssl", Severity::Medium, Some(6.9));
    assert!(vuln::component_has_cvss_at_least(&c, &[v], 7.0).is_none());
}

// ─── 8. Policy evaluator state transitions ──────────────────────────────────

#[test]
fn policy_pipeline_empty_components_emits_no_violations() {
    let p = Policy {
        uuid: Uuid::new_v4(),
        name: "P".into(),
        violation_state: ViolationState::Fail,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::LicenseDeny {
            deny: vec!["GPL-3.0".into()],
        }],
    };
    let viols = evaluate_pipeline(&[p], &[], &[], Utc::now());
    assert!(viols.is_empty());
}

#[test]
fn policy_operator_all_with_single_match_fires() {
    // ALL with one condition that matches → fires.
    let p = Policy {
        uuid: Uuid::new_v4(),
        name: "P".into(),
        violation_state: ViolationState::Warn,
        operator: Operator::All,
        conditions: vec![PolicyCondition::LicenseDeny {
            deny: vec!["GPL-3.0".into()],
        }],
    };
    let mut c = mk_comp("foo");
    c.license = Some("GPL-3.0".into());
    let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
    assert_eq!(viols.len(), 1);
    assert_eq!(viols[0].violation_state, ViolationState::Warn);
}

#[test]
fn policy_age_no_published_at_no_violation_even_if_zero_threshold() {
    let c = mk_comp("x"); // published_at = None.
    let now = Utc::now();
    assert!(age::violates(&c, 0, now).is_none());
}

#[test]
fn policy_age_published_in_future_no_violation() {
    let mut c = mk_comp("x");
    c.published_at = Some(Utc::now() + Duration::days(100));
    assert!(age::violates(&c, 0, Utc::now()).is_none());
}

#[test]
fn policy_coordinates_wildcard_name_matches_anything() {
    let c = mk_comp("anything");
    assert!(coordinates::violates(&c, None, "*", None).is_some());
}

#[test]
fn policy_license_allow_uppercase_lowercase_equivalence() {
    let mut c = mk_comp("x");
    c.license = Some("Apache-2.0".into());
    // Case-insensitive match required.
    assert!(license::violates_allow(&c, &["apache-2.0".into()]).is_none());
}

// ─── 9. Component graph traversal ───────────────────────────────────────────

#[test]
fn dependency_tree_diamond_does_not_double_visit() {
    // A→B, A→C, B→D, C→D — D should appear once.
    let comps = vec![
        Component {
            id: "a".into(),
            name: "a".into(),
            version: "1".into(),
            purl: None,
            license: None,
            component_type: ComponentType::Application,
            dependencies: vec!["b".into(), "c".into()],
        },
        Component {
            id: "b".into(),
            name: "b".into(),
            version: "1".into(),
            purl: None,
            license: None,
            component_type: ComponentType::Library,
            dependencies: vec!["d".into()],
        },
        Component {
            id: "c".into(),
            name: "c".into(),
            version: "1".into(),
            purl: None,
            license: None,
            component_type: ComponentType::Library,
            dependencies: vec!["d".into()],
        },
        Component {
            id: "d".into(),
            name: "d".into(),
            version: "1".into(),
            purl: None,
            license: None,
            component_type: ComponentType::Library,
            dependencies: vec![],
        },
    ];
    let tree = build_dependency_tree(&comps, "a");
    let mut deps = find_transitive_deps(&tree, "a");
    deps.sort();
    deps.dedup();
    assert_eq!(deps, vec!["b".to_string(), "c".to_string(), "d".to_string()]);
    // Length matches deduped set.
    let deps2 = find_transitive_deps(&tree, "a");
    let unique: std::collections::HashSet<_> = deps2.iter().collect();
    assert_eq!(unique.len(), deps2.len());
}

#[test]
fn find_by_license_skips_none_license() {
    let comps = vec![
        Component {
            id: "a".into(),
            name: "a".into(),
            version: "1".into(),
            purl: None,
            license: None,
            component_type: ComponentType::Library,
            dependencies: vec![],
        },
        Component {
            id: "b".into(),
            name: "b".into(),
            version: "1".into(),
            purl: None,
            license: Some("MIT".into()),
            component_type: ComponentType::Library,
            dependencies: vec![],
        },
    ];
    let mit = find_by_license(&comps, "MIT");
    assert_eq!(mit.len(), 1);
}

#[test]
fn count_by_type_empty_slice_is_empty_map() {
    let counts = count_by_type(&[]);
    assert!(counts.is_empty());
}

// ─── 10. Version compare edges ──────────────────────────────────────────────

#[test]
fn version_compare_build_metadata_ignored() {
    use std::cmp::Ordering;
    assert_eq!(version_compare("1.0.0+a", "1.0.0+b"), Ordering::Equal);
}

#[test]
fn version_compare_longer_release_is_greater() {
    use std::cmp::Ordering;
    // Token-vec comparison: shorter prefix is Less than its longer-suffix counterpart.
    assert_eq!(version_compare("1.0", "1.0.0"), Ordering::Less);
    assert_eq!(version_compare("1.0.0", "1.0"), Ordering::Greater);
}

// ─── 11. Component identity (purl vs gnv) ───────────────────────────────────

#[test]
fn identity_purl_mismatch_falls_back_to_gnv_match() {
    let pu = Uuid::new_v4();
    let mut a = ComponentRecord::new(pu, "lodash", "4.17.21");
    a.purl = Some("pkg:npm/lodash@4.17.21".into());
    a.group = Some("npm".into());
    let mut b = ComponentRecord::new(pu, "lodash", "4.17.21");
    // Different purl but matching gnv — should still match.
    b.purl = Some("pkg:npm/different@4.17.21".into());
    b.group = Some("npm".into());
    assert!(
        ComponentIdentity::from_record(&a).matches(&ComponentIdentity::from_record(&b)),
        "purl mismatch should fall back to (group,name,version)"
    );
}

#[test]
fn version_index_dedups_same_version() {
    let p = Uuid::new_v4();
    let recs = vec![
        ComponentRecord::new(p, "lodash", "1.0.0"),
        ComponentRecord::new(p, "lodash", "1.0.0"),
        ComponentRecord::new(p, "lodash", "1.0.0"),
    ];
    let idx = version_index(&recs);
    assert_eq!(idx["lodash"], vec!["1.0.0"]);
}

#[test]
fn projects_for_component_empty_when_name_missing() {
    let p1 = Uuid::new_v4();
    let recs = vec![ComponentRecord::new(p1, "lodash", "1")];
    assert!(projects_for_component(&recs, "no-such").is_empty());
}

// ─── 12. Vuln-intel merge / NVD ─────────────────────────────────────────────

#[test]
fn merge_advisories_equal_cvss_keeps_first_seen() {
    // Tie-break: existing wins (only strictly-greater replaces).
    let merged = merge_advisories(vec![
        mk_vuln("a", Severity::High, Some(7.0)),
        mk_vuln("a", Severity::High, Some(7.0)),
    ]);
    assert_eq!(merged.len(), 1);
}

#[test]
fn nvd_parse_empty_input_is_error() {
    let err = nvd::parse_cves_response(b"").unwrap_err();
    assert!(matches!(err, nvd::NvdError::Json(_)));
}

#[test]
fn nvd_v30_fallback_when_no_v31() {
    let blob = br#"{"vulnerabilities":[{"cve":{
        "id":"CVE-X","descriptions":[{"lang":"en","value":"x"}],
        "metrics":{"cvssMetricV30":[{"cvssData":{"baseScore":5.0,"vectorString":"CVSS:3.0/X"}}]},
        "weaknesses":[],"references":[],"configurations":[]
    }}]}"#;
    let v = nvd::parse_cves_response(blob).unwrap();
    assert_eq!(v[0].cvss_v3_base, Some(5.0));
    assert_eq!(v[0].severity, Severity::Medium);
}

// ─── 13. OSV / GHSA edges ───────────────────────────────────────────────────

#[test]
fn osv_introduced_zero_omits_lower_bound() {
    // {introduced: "0"} should NOT produce ">=0"; the upper bound stands alone.
    let blob = br#"{"id":"OSV-1","aliases":[],"affected":[
        {"package":{"ecosystem":"npm","name":"x"},
         "ranges":[{"type":"SEMVER","events":[{"introduced":"0"},{"fixed":"2.0.0"}]}]}],
        "references":[]}"#;
    let v = osv::parse_advisory(blob).unwrap();
    assert_eq!(v.affected[0].vers, "<2.0.0");
}

#[test]
fn osv_last_affected_produces_inclusive_upper() {
    let blob = br#"{"id":"OSV-2","aliases":[],"affected":[
        {"package":{"ecosystem":"pypi","name":"y"},
         "ranges":[{"type":"ECOSYSTEM","events":[{"introduced":"1.0"},{"last_affected":"2.5"}]}]}],
        "references":[]}"#;
    let v = osv::parse_advisory(blob).unwrap();
    assert_eq!(v.affected[0].purl_type, "pypi");
    assert_eq!(v.affected[0].vers, ">=1.0 <=2.5");
}

#[test]
fn ghsa_critical_uppercase_severity_maps_to_critical() {
    let blob = r#"{"data":{"securityAdvisories":{"nodes":[{
      "ghsaId":"GHSA-z","summary":"t","description":"d","severity":"CRITICAL",
      "cwes":{"nodes":[]},"cvss":{"score":9.9},
      "publishedAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:00:00Z",
      "identifiers":[{"type":"GHSA","value":"GHSA-z"}],"references":[],
      "vulnerabilities":{"nodes":[]}}]}}}"#;
    let v = ghsa::parse_graphql_response(blob.as_bytes()).unwrap();
    assert_eq!(v[0].severity, Severity::Critical);
}

#[test]
fn ghsa_unknown_severity_maps_to_unassigned() {
    let blob = r#"{"data":{"securityAdvisories":{"nodes":[{
      "ghsaId":"GHSA-a","summary":"t","description":"d","severity":"BOGUS",
      "cwes":{"nodes":[]},"cvss":{"score":null},
      "publishedAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:00:00Z",
      "identifiers":[{"type":"GHSA","value":"GHSA-a"}],"references":[],
      "vulnerabilities":{"nodes":[]}}]}}}"#;
    let v = ghsa::parse_graphql_response(blob.as_bytes()).unwrap();
    assert_eq!(v[0].severity, Severity::Unassigned);
}

#[test]
fn ghsa_missing_data_block_is_explicit_error() {
    let err = ghsa::parse_graphql_response(b"{}").unwrap_err();
    assert!(matches!(err, GhsaError::MissingData));
}

#[test]
fn snyk_severity_via_score_overrides_level_text() {
    // Snyk parser uses score when present; "low" level + 9.9 score → Critical.
    let blob = br#"{"data":[{"id":"SNYK-X","attributes":{
      "title":"t","description":"d","severities":[{"level":"low","score":9.9}],
      "problems":[],"coordinates":[],"created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}}]}"#;
    let v = snyk::parse_response(blob).unwrap();
    assert_eq!(v[0].severity, Severity::Critical);
}

// ─── 14. EPSS edges ─────────────────────────────────────────────────────────

#[test]
fn epss_zero_score_accepted() {
    let v = epss::parse_csv("CVE-X,0.0,0.0\n").unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].score, 0.0);
}

#[test]
fn epss_score_exactly_one_accepted() {
    let v = epss::parse_csv("CVE-Y,1.0,1.0\n").unwrap();
    assert_eq!(v[0].score, 1.0);
}

#[test]
fn epss_negative_score_rejected() {
    let err = epss::parse_csv("CVE-Y,-0.1,0.5\n").unwrap_err();
    assert!(matches!(err, EpssError::OutOfRange(_)));
}

#[test]
fn epss_join_misses_leave_intel_clean() {
    let idx: HashMap<String, EpssScore> = build_index(vec![EpssScore {
        cve_id: "CVE-OTHER".into(),
        score: 0.5,
        percentile: 0.9,
    }]);
    let mut intels = vec![mk_vuln("foo", Severity::High, Some(7.5))];
    let hits = join_in_place(&mut intels, &idx);
    assert_eq!(hits, 0);
    assert!(intels[0].epss_score.is_none());
}

// ─── 15. Portfolio risk roll-up ─────────────────────────────────────────────

#[test]
fn portfolio_risk_unknown_uuid_returns_empty_counts() {
    let r = ProjectRisk::compute(Uuid::new_v4(), &[], &[]);
    assert_eq!(r.total_components, 0);
    assert_eq!(r.inherited_risk_score, 0.0);
}

#[test]
fn portfolio_risk_unassigned_severity_still_weighted_at_five() {
    let p = Uuid::new_v4();
    let mut c = ComponentRecord::new(p, "x", "1.0.0");
    c.purl = Some("pkg:npm/x@1".into());
    let v = mk_vuln("x", Severity::Unassigned, None);
    let r = ProjectRisk::compute(p, &[c], &[v]);
    assert_eq!(r.unassigned, 1);
    assert_eq!(r.inherited_risk_score, 5.0);
}

#[test]
fn vulnerable_trend_empty_series_is_empty() {
    let t = vulnerable_trend(&[]);
    assert!(t.is_empty());
}

#[test]
fn portfolio_snapshot_empty_projects_aggregates_zero() {
    let s = PortfolioSnapshot::take(&[], &[], &[], Utc::now());
    assert_eq!(s.total_vulnerable(), 0);
    assert_eq!(s.total_critical(), 0);
}

// ─── 16. Notifications rule + payload edges ─────────────────────────────────

#[test]
fn rule_min_level_at_exact_threshold_matches() {
    let r = NotificationRule {
        uuid: Uuid::new_v4(),
        name: "r".into(),
        enabled: true,
        notify_on: vec![NotificationGroup::NewVulnerability],
        min_level: NotificationLevel::Error,
        publisher: PublisherKind::Console,
    };
    let n = Notification {
        group: NotificationGroup::NewVulnerability,
        level: NotificationLevel::Error,
        title: "t".into(),
        content: "c".into(),
        payload: None,
    };
    assert!(rule_matches(&r, &n));
}

#[test]
fn jira_payload_label_is_lowercase_group_name() {
    let n = Notification {
        group: NotificationGroup::PolicyViolation,
        level: NotificationLevel::Warning,
        title: "t".into(),
        content: "c".into(),
        payload: None,
    };
    let p = jira_issue_payload("KEY", &n);
    assert_eq!(p["fields"]["labels"][0], "policyviolation");
}

#[test]
fn webhook_envelope_uppercases_level() {
    let n = Notification {
        group: NotificationGroup::BomProcessed,
        level: NotificationLevel::Warning,
        title: "t".into(),
        content: "c".into(),
        payload: None,
    };
    let p = webhook_payload(&n);
    assert_eq!(p["notification"]["level"], "WARNING");
    assert_eq!(p["notification"]["group"], "BomProcessed");
}

#[test]
fn slack_and_teams_share_palette() {
    let n = Notification {
        group: NotificationGroup::PolicyViolation,
        level: NotificationLevel::Error,
        title: "t".into(),
        content: "c".into(),
        payload: None,
    };
    let s = slack_payload(&n);
    let t = teams_payload(&n);
    // Slack uses hex with `#`, Teams without — but the 6-digit core matches.
    let s_color = s["attachments"][0]["color"].as_str().unwrap();
    let t_color = t["themeColor"].as_str().unwrap();
    assert_eq!(
        s_color.trim_start_matches('#').to_ascii_uppercase(),
        t_color.to_ascii_uppercase()
    );
}

#[test]
fn mail_subject_trims_prefix_whitespace() {
    let n = Notification {
        group: NotificationGroup::ProjectAuditChange,
        level: NotificationLevel::Informational,
        title: "T".into(),
        content: "C".into(),
        payload: None,
    };
    let s = build_subject("  px  ", &n);
    assert_eq!(s, "[px] Informational: T");
    let body = build_text_body(&n);
    assert!(body.contains("ProjectAuditChange"));
}

// ─── 17. Serde round-trip integrity ─────────────────────────────────────────

#[test]
fn project_serde_round_trip_full() {
    let p = Project::new("my-app", Some("1.0.0".into()));
    let j = serde_json::to_string(&p).unwrap();
    let back: Project = serde_json::from_str(&j).unwrap();
    assert_eq!(p, back);
}

#[test]
fn component_record_serde_round_trip() {
    let mut c = ComponentRecord::new(Uuid::new_v4(), "lodash", "4.17.21");
    c.purl = Some("pkg:npm/lodash@4.17.21".into());
    c.license = Some("MIT".into());
    c.hash_sha256 = Some("deadbeef".repeat(8));
    let j = serde_json::to_string(&c).unwrap();
    let back: ComponentRecord = serde_json::from_str(&j).unwrap();
    assert_eq!(c, back);
}

#[test]
fn sbom_format_lower_snake_serde() {
    let f = SbomFormat::CycloneDx;
    let s = serde_json::to_string(&f).unwrap();
    assert_eq!(s, "\"cyclone_dx\"");
    let back: SbomFormat = serde_json::from_str(&s).unwrap();
    assert_eq!(f, back);
}

#[test]
fn analysis_state_full_matrix_round_trip() {
    for st in [
        AnalysisState::NotSet,
        AnalysisState::Exploitable,
        AnalysisState::InTriage,
        AnalysisState::Resolved,
        AnalysisState::FalsePositive,
        AnalysisState::NotAffected,
    ] {
        let j = serde_json::to_string(&st).unwrap();
        let back: AnalysisState = serde_json::from_str(&j).unwrap();
        assert_eq!(st, back);
    }
}

#[test]
fn sbom_minimal_serde_round_trip_with_empty_components() {
    let s = Sbom {
        id: Uuid::new_v4(),
        name: "x".into(),
        version: "1".into(),
        format: SbomFormat::Spdx,
        components: vec![],
        created_at: Utc::now(),
    };
    let j = serde_json::to_string(&s).unwrap();
    let back: Sbom = serde_json::from_str(&j).unwrap();
    assert_eq!(s, back);
}
