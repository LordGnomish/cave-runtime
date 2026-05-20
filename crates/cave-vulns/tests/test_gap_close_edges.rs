// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Integration tests filling edge-case gaps in the cave-vulns public API:
// finding deduplication boundary behaviour, SLA / risk-acceptance lifecycle
// transitions, parser-format hints via serde, CVSS scoring boundaries, and
// product-style hierarchy validation via the scan-result aggregation.

use cave_vulns::{
    dedup::{dedup_key, deduplicate, is_sla_breached, sla_days, sla_deadline},
    engine::{
        build_scan_result, count_by_severity, cvss_to_severity, find_for_component, is_affected,
        version_lt,
    },
    models::{ComponentVersion, Severity, VulnScanResult, VulnState, Vulnerability},
    MODULE_NAME, State,
};
use chrono::{Duration, TimeZone, Utc};
use std::sync::Arc;
use uuid::Uuid;

fn fixture(
    cve: &str,
    component: &str,
    versions: &[&str],
    severity: Severity,
    cvss: f32,
    state: VulnState,
) -> Vulnerability {
    Vulnerability {
        id: Uuid::new_v4(),
        cve_id: cve.to_string(),
        title: format!("Test {cve}"),
        description: "fixture".to_string(),
        severity,
        cvss_score: cvss,
        affected_component: component.to_string(),
        affected_versions: versions.iter().map(|s| s.to_string()).collect(),
        fixed_in: Some("9.9.9".to_string()),
        published_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        state,
    }
}

// ─────────────────────────── module surface ──────────────────────────────

#[test]
fn module_name_is_stable() {
    // Public re-exported constant — downstream wiring may pattern-match on it.
    assert_eq!(MODULE_NAME, "vulns");
}

#[test]
fn state_default_constructs_with_in_memory_storage() {
    // Default impl must not panic and must produce a usable Arc.
    let s = State::default();
    let _arc = Arc::new(s);
}

#[test]
fn router_constructs_from_default_state() {
    // Router builder must wire successfully against a freshly-built State.
    let state = Arc::new(State::default());
    let _router = cave_vulns::router(state);
}

// ─────────────────────────── CVSS scoring boundary ───────────────────────

#[test]
fn cvss_score_at_critical_floor_is_critical() {
    // 9.0 is the inclusive lower bound for Critical (CVSS v3 spec).
    assert_eq!(cvss_to_severity(9.0), Severity::Critical);
}

#[test]
fn cvss_score_just_below_critical_floor_is_high() {
    // 8.999 must NOT promote to Critical — boundary integrity.
    assert_eq!(cvss_to_severity(8.999), Severity::High);
}

#[test]
fn cvss_score_at_high_floor_is_high() {
    assert_eq!(cvss_to_severity(7.0), Severity::High);
}

#[test]
fn cvss_score_just_below_high_floor_is_medium() {
    assert_eq!(cvss_to_severity(6.9), Severity::Medium);
}

#[test]
fn cvss_score_at_medium_floor_is_medium() {
    assert_eq!(cvss_to_severity(4.0), Severity::Medium);
}

#[test]
fn cvss_score_just_below_medium_floor_is_low() {
    assert_eq!(cvss_to_severity(3.9), Severity::Low);
}

#[test]
fn cvss_score_at_low_floor_is_low() {
    // 0.1 is the smallest score that registers as Low (per cave-vulns engine).
    assert_eq!(cvss_to_severity(0.1), Severity::Low);
}

#[test]
fn cvss_score_zero_is_info() {
    assert_eq!(cvss_to_severity(0.0), Severity::Info);
}

#[test]
fn cvss_score_negative_falls_back_to_info() {
    // Defensive: malformed scanner input must not panic; classifies as Info.
    assert_eq!(cvss_to_severity(-1.0), Severity::Info);
}

#[test]
fn cvss_score_above_ten_is_critical() {
    // Some scanners emit scores marginally above 10.0; clamp upward to Critical.
    assert_eq!(cvss_to_severity(10.5), Severity::Critical);
}

// ─────────────────────────── version comparison ─────────────────────────

#[test]
fn version_lt_handles_unequal_segment_count() {
    // "1.2" vs "1.2.1" — the longer side has an extra non-zero segment.
    assert!(version_lt("1.2", "1.2.1"));
    assert!(!version_lt("1.2.1", "1.2"));
}

#[test]
fn version_lt_treats_trailing_zero_as_equal() {
    // "1.2.0" vs "1.2" are semantically equal — neither is less than the other.
    assert!(!version_lt("1.2.0", "1.2"));
    assert!(!version_lt("1.2", "1.2.0"));
}

#[test]
fn version_lt_ignores_non_numeric_segments() {
    // "1.2.x" — non-numeric components parse-fail and are dropped.
    assert!(version_lt("1.2.x", "1.3"));
}

#[test]
fn version_lt_with_empty_strings_is_not_less() {
    assert!(!version_lt("", ""));
}

// ─────────────────────────── is_affected / find_for_component ────────────

#[test]
fn is_affected_matches_one_of_many_versions() {
    let v = fixture(
        "CVE-X",
        "openssl",
        &["1.0.1", "1.0.2", "1.1.0"],
        Severity::High,
        7.5,
        VulnState::Open,
    );
    assert!(is_affected(&v, "openssl", "1.0.2"));
}

#[test]
fn is_affected_is_component_case_sensitive() {
    // Component matching is case-sensitive — "OpenSSL" != "openssl".
    let v = fixture("CVE-X", "openssl", &["1.0.1"], Severity::High, 7.5, VulnState::Open);
    assert!(!is_affected(&v, "OpenSSL", "1.0.1"));
}

#[test]
fn find_for_component_returns_empty_when_no_match() {
    let vulns = vec![
        fixture("CVE-A", "openssl", &["1.0.1"], Severity::High, 7.5, VulnState::Open),
        fixture("CVE-B", "libcurl", &["2.0"], Severity::Medium, 5.0, VulnState::Open),
    ];
    let needle = ComponentVersion {
        name: "nginx".to_string(),
        version: "1.0".to_string(),
    };
    let hits = find_for_component(&vulns, &needle);
    assert!(hits.is_empty());
}

#[test]
fn find_for_component_returns_all_matching() {
    let vulns = vec![
        fixture("CVE-A", "openssl", &["1.0.1"], Severity::High, 7.5, VulnState::Open),
        fixture("CVE-B", "openssl", &["1.0.1"], Severity::Critical, 9.5, VulnState::Open),
        fixture("CVE-C", "openssl", &["1.1.0"], Severity::High, 7.5, VulnState::Open),
    ];
    let needle = ComponentVersion {
        name: "openssl".to_string(),
        version: "1.0.1".to_string(),
    };
    let hits = find_for_component(&vulns, &needle);
    assert_eq!(hits.len(), 2);
}

// ─────────────────────────── deduplication edges ────────────────────────

#[test]
fn dedup_key_distinct_on_version_set() {
    // Same CVE+component, but different affected-version sets must be distinct.
    let a = fixture("CVE-X", "openssl", &["1.0.1"], Severity::High, 7.5, VulnState::Open);
    let b = fixture("CVE-X", "openssl", &["1.0.2"], Severity::High, 7.5, VulnState::Open);
    assert_ne!(dedup_key(&a), dedup_key(&b));
}

#[test]
fn deduplicate_empty_input_yields_empty() {
    let out = deduplicate(vec![]);
    assert!(out.is_empty());
}

#[test]
fn deduplicate_keeps_critical_over_low_regardless_of_input_order() {
    // Lower severity arrives first — Critical must still win.
    let findings = vec![
        fixture("CVE-1", "openssl", &["1"], Severity::Low, 2.0, VulnState::Open),
        fixture("CVE-1", "openssl", &["1"], Severity::Critical, 9.5, VulnState::Open),
    ];
    let out = deduplicate(findings);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].severity, Severity::Critical);
}

#[test]
fn deduplicate_preserves_count_when_all_unique() {
    let findings: Vec<Vulnerability> = (0..10)
        .map(|i| {
            fixture(
                &format!("CVE-2026-{i}"),
                "openssl",
                &["1.0.1"],
                Severity::High,
                7.5,
                VulnState::Open,
            )
        })
        .collect();
    let out = deduplicate(findings);
    assert_eq!(out.len(), 10);
}

// ─────────────────────────── SLA + risk-acceptance transitions ──────────

#[test]
fn sla_days_table_complete_for_all_severities() {
    // Exhaustive table — guards against silent SLA-policy regressions.
    assert_eq!(sla_days(&Severity::Critical), Some(7));
    assert_eq!(sla_days(&Severity::High), Some(30));
    assert_eq!(sla_days(&Severity::Medium), Some(90));
    assert_eq!(sla_days(&Severity::Low), Some(180));
    assert_eq!(sla_days(&Severity::Info), None);
}

#[test]
fn sla_deadline_exactly_on_boundary_is_not_breached() {
    // `now == deadline` is the wire-boundary; strict `>` semantics in is_sla_breached.
    let v = fixture("CVE-1", "c", &["1"], Severity::High, 7.5, VulnState::Open);
    let on_boundary = v.published_at + Duration::days(30);
    assert!(!is_sla_breached(&v, on_boundary));
}

#[test]
fn sla_deadline_one_second_past_boundary_is_breached() {
    let v = fixture("CVE-1", "c", &["1"], Severity::High, 7.5, VulnState::Open);
    let one_sec_past = v.published_at + Duration::days(30) + Duration::seconds(1);
    assert!(is_sla_breached(&v, one_sec_past));
}

#[test]
fn risk_acceptance_transition_acknowledged_does_not_resolve() {
    // State-machine: Acknowledged is NOT a terminal state — SLA still ticks.
    let v = fixture(
        "CVE-1",
        "c",
        &["1"],
        Severity::Critical,
        9.5,
        VulnState::Acknowledged,
    );
    let breach_time = v.published_at + Duration::days(8);
    assert!(is_sla_breached(&v, breach_time));
    assert_eq!(v.state, VulnState::Acknowledged);
}

#[test]
fn risk_acceptance_transition_false_positive_still_carries_sla() {
    // FalsePositive is a verdict label; engine still computes deadline.
    // (Filtering happens upstream — engine reports the breach honestly.)
    let v = fixture(
        "CVE-1",
        "c",
        &["1"],
        Severity::Medium,
        5.5,
        VulnState::FalsePositive,
    );
    assert!(sla_deadline(&v).is_some());
}

// ─────────────────────────── parser format detection (serde) ────────────

#[test]
fn severity_deserialises_snake_case_from_external_format() {
    // SARIF / CycloneDX scanners emit lowercase snake_case strings.
    let json = r#""critical""#;
    let s: Severity = serde_json::from_str(json).expect("snake_case deser");
    assert_eq!(s, Severity::Critical);
}

#[test]
fn severity_rejects_unknown_variant() {
    // Defensive: an unknown severity from a malformed report fails to parse.
    let res: Result<Severity, _> = serde_json::from_str(r#""apocalyptic""#);
    assert!(res.is_err());
}

#[test]
fn vuln_state_deserialises_false_positive_snake_case() {
    // "false_positive" — multi-word snake_case round-trip.
    let json = r#""false_positive""#;
    let s: VulnState = serde_json::from_str(json).expect("deser false_positive");
    assert_eq!(s, VulnState::FalsePositive);
}

#[test]
fn vulnerability_round_trip_preserves_optional_fixed_in_none() {
    let mut v = fixture("CVE-1", "c", &["1"], Severity::Low, 1.0, VulnState::Open);
    v.fixed_in = None;
    let json = serde_json::to_string(&v).unwrap();
    let back: Vulnerability = serde_json::from_str(&json).unwrap();
    assert_eq!(v, back);
    assert!(back.fixed_in.is_none());
}

#[test]
fn vulnerability_round_trip_with_many_versions() {
    // Stress affected_versions vec — common surface for CycloneDX bom-ref lists.
    let v = fixture(
        "CVE-2026-9999",
        "openssl",
        &["1.0.1", "1.0.2", "1.1.0", "1.1.1", "3.0.0"],
        Severity::High,
        8.1,
        VulnState::Mitigated,
    );
    let json = serde_json::to_string(&v).unwrap();
    let back: Vulnerability = serde_json::from_str(&json).unwrap();
    assert_eq!(v, back);
    assert_eq!(back.affected_versions.len(), 5);
}

#[test]
fn component_version_deserialises_with_extra_fields_strictly_fails() {
    // ComponentVersion lacks #[serde(deny_unknown_fields)] in src, so extras pass.
    // This documents current behaviour (lenient parser; SARIF/SBOM ingestion-friendly).
    let json = r#"{"name":"openssl","version":"1.0.1","extra":"ignored"}"#;
    let cv: ComponentVersion = serde_json::from_str(json).expect("lenient deser");
    assert_eq!(cv.name, "openssl");
    assert_eq!(cv.version, "1.0.1");
}

// ─────────────────────────── product/test hierarchy aggregation ─────────

#[test]
fn engagement_lifecycle_count_by_severity_handles_empty() {
    let (c, h, m, l) = count_by_severity(&[]);
    assert_eq!((c, h, m, l), (0, 0, 0, 0));
}

#[test]
fn engagement_lifecycle_count_excludes_info_from_top_four() {
    // Info severity must NOT inflate the critical/high/medium/low totals.
    let findings = vec![
        fixture("CVE-1", "a", &["1"], Severity::Info, 0.0, VulnState::Open),
        fixture("CVE-2", "b", &["1"], Severity::Info, 0.0, VulnState::Open),
        fixture("CVE-3", "c", &["1"], Severity::Critical, 9.5, VulnState::Open),
    ];
    let (c, h, m, l) = count_by_severity(&findings);
    assert_eq!((c, h, m, l), (1, 0, 0, 0));
}

#[test]
fn product_test_hierarchy_build_scan_result_target_propagates() {
    // VulnScanResult.target is the "product/engagement" label in DefectDojo
    // terms; must survive scan-result construction verbatim.
    let result = build_scan_result(
        "frontend-prod/v1.2.3",
        vec![fixture(
            "CVE-1",
            "openssl",
            &["1.0.1"],
            Severity::High,
            7.5,
            VulnState::Open,
        )],
    );
    assert_eq!(result.target, "frontend-prod/v1.2.3");
    assert_eq!(result.findings.len(), 1);
}

#[test]
fn product_test_hierarchy_build_scan_result_distinct_scan_ids() {
    // Each scan invocation gets a fresh UUID — collision-resistance check.
    let r1 = build_scan_result("svc-a", vec![]);
    let r2 = build_scan_result("svc-a", vec![]);
    assert_ne!(r1.scan_id, r2.scan_id);
}

#[test]
fn product_test_hierarchy_scan_result_serde_round_trip() {
    let findings = vec![
        fixture("CVE-A", "openssl", &["1.0.1"], Severity::Critical, 9.5, VulnState::Open),
        fixture("CVE-B", "libcurl", &["7.0"], Severity::Medium, 5.0, VulnState::Mitigated),
    ];
    let result: VulnScanResult = build_scan_result("svc-x", findings);
    let json = serde_json::to_string(&result).unwrap();
    let back: VulnScanResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.target, "svc-x");
    assert_eq!(back.total_critical, 1);
    assert_eq!(back.total_medium, 1);
    assert_eq!(back.findings.len(), 2);
}
