// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for cave-scan public surface (engine + coverage).

use cave_scan::coverage::{parse_cobertura, parse_lcov};
use cave_scan::engine::{
    build_result, filter_by_min_severity, match_keyword, scan_content, severity_rank,
};
use cave_scan::models::{Finding, FindingSeverity, RuleType, ScanRule};
use uuid::Uuid;

fn rule(pattern: &str, severity: FindingSeverity) -> ScanRule {
    ScanRule {
        id: Uuid::new_v4(),
        name: format!("rule-{pattern}"),
        description: "integration".to_string(),
        pattern: pattern.to_string(),
        rule_type: RuleType::Keyword,
        severity,
        enabled: true,
    }
}

fn finding(severity: FindingSeverity) -> Finding {
    Finding {
        id: Uuid::new_v4(),
        rule_id: Uuid::new_v4(),
        rule_name: "r".to_string(),
        file_path: "p".to_string(),
        line_number: 1,
        matched_text: "m".to_string(),
        severity,
        message: "msg".to_string(),
    }
}

#[test]
fn integration_pipeline_scan_then_filter_then_build() {
    let rules = vec![
        rule("password", FindingSeverity::Critical),
        rule("todo", FindingSeverity::Info),
    ];
    let content = "let password = \"x\";\n// todo later\nlet token = 1;";
    let findings = scan_content(&rules, content, "src/x.rs");
    assert_eq!(findings.len(), 2);

    let major_plus = filter_by_min_severity(&findings, &FindingSeverity::Major);
    assert_eq!(major_plus.len(), 1);

    let result = build_result("svc", findings, rules.len(), 1);
    assert_eq!(result.findings.len(), 2);
    assert_eq!(result.target, "svc");
}

#[test]
fn integration_severity_rank_matches_filter_logic() {
    let f = vec![
        finding(FindingSeverity::Critical),
        finding(FindingSeverity::Major),
        finding(FindingSeverity::Minor),
        finding(FindingSeverity::Info),
    ];
    let above_minor = filter_by_min_severity(&f, &FindingSeverity::Minor);
    for kept in &above_minor {
        assert!(severity_rank(&kept.severity) >= severity_rank(&FindingSeverity::Minor));
    }
    assert_eq!(above_minor.len(), 3);
}

#[test]
fn integration_match_keyword_records_file_path() {
    let r = rule("secret", FindingSeverity::Major);
    let findings = match_keyword(&r, "let secret = 1;", "deep/nested/path/file.rs");
    assert_eq!(findings[0].file_path, "deep/nested/path/file.rs");
}

#[test]
fn integration_lcov_to_summary_correct_overall() {
    let lcov = "TN:a\nLF:200\nLH:160\nend_of_record\nTN:b\nLF:300\nLH:90\nend_of_record\n";
    let report = parse_lcov(lcov);
    assert_eq!(report.total_lines, 500);
    assert_eq!(report.covered_lines, 250);
    assert!((report.coverage_percent - 50.0).abs() < 0.01);
}

#[test]
fn integration_cobertura_with_three_packages_extracts_all() {
    let xml = r#"<coverage line-rate="0.75"><package name="p1"/><package name="p2"/><package name="p3"/></coverage>"#;
    let report = parse_cobertura(xml);
    assert_eq!(report.files.len(), 3);
    assert!((report.coverage_percent - 75.0).abs() < 0.01);
}

#[test]
fn integration_finding_serde_roundtrip_via_value() {
    let f = finding(FindingSeverity::Critical);
    let v = serde_json::to_value(&f).unwrap();
    let back: Finding = serde_json::from_value(v).unwrap();
    assert_eq!(f, back);
}
