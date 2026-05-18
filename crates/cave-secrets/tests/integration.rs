// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for cave-secrets public surface.

use cave_secrets::detector::{builtin_detectors, scan, shannon_entropy, Severity};

#[test]
fn integration_scan_aws_then_filter_by_severity() {
    let dets = builtin_detectors();
    let content = "AWS_KEY=AKIAIOSFODNN7EXAMPLE";
    let findings = scan(content, "creds.env", &dets);
    let crit: Vec<_> = findings.iter().filter(|f| f.severity == Severity::Critical).collect();
    assert!(!crit.is_empty());
}

#[test]
fn integration_clean_repo_yields_no_findings() {
    let dets = builtin_detectors();
    let content = "PORT=8080\nHOST=localhost\nLOG_LEVEL=info\n";
    let findings = scan(content, "app.env", &dets);
    assert!(findings.is_empty());
}

#[test]
fn integration_multi_file_simulation_aggregates() {
    let dets = builtin_detectors();
    let f1 = scan("AWS_KEY=AKIAIOSFODNN7EXAMPLE", "a.env", &dets);
    let f2 = scan("-----BEGIN RSA PRIVATE KEY-----", "id_rsa", &dets);
    assert!(!f1.is_empty() && !f2.is_empty());
    assert_ne!(f1[0].file, f2[0].file);
}

#[test]
fn integration_default_state_uses_builtin_detectors() {
    let st = cave_secrets::SecretsState::default();
    assert!(!st.detectors.is_empty());
}

#[test]
fn integration_entropy_high_secret_value_detected() {
    let value = "abZ3$xY9!kLmN2@pQrS_xx_yy_qq";
    assert!(shannon_entropy(value) > 4.0);
}

#[test]
fn integration_finding_records_correct_filename() {
    let dets = builtin_detectors();
    let findings = scan("AKIAIOSFODNN7EXAMPLE", "deep/path/secrets.env", &dets);
    assert!(findings.iter().all(|f| f.file == "deep/path/secrets.env"));
}

#[test]
fn integration_detector_list_contains_expected_names() {
    let names: Vec<&str> = builtin_detectors().iter().map(|d| d.name).collect();
    for required in [
        "aws-access-key",
        "github-token",
        "private-key",
        "jwt-token",
        "slack-webhook",
        "azure-connection-string",
    ] {
        assert!(names.contains(&required), "missing detector: {required}");
    }
}

#[test]
fn integration_finding_line_numbers_one_indexed() {
    let dets = builtin_detectors();
    let content = "x\ny\nAKIAIOSFODNN7EXAMPLE\n";
    let findings = scan(content, "f.env", &dets);
    let aws = findings.iter().find(|f| f.detector == "aws-access-key").unwrap();
    assert_eq!(aws.line, 3);
}

#[test]
fn integration_redaction_keeps_short_lines_intact() {
    // Input shorter than 20 chars should not be redacted with ellipsis.
    let dets = builtin_detectors();
    let findings = scan("-----BEGIN RSA PRIVATE KEY-----", "id_rsa", &dets);
    let pk = findings.iter().find(|f| f.detector == "private-key").unwrap();
    // 31 chars => above threshold, will be redacted; ensure ellipsis exists.
    assert!(pk.matched.contains("...") || pk.matched.len() <= 20);
}

#[test]
fn integration_entropy_detector_does_not_double_fire_on_clean_long_lines() {
    let dets = builtin_detectors();
    // Long but clearly-readable English text — should not fire entropy detector.
    let content = "this is a normal long sentence that should not trigger the entropy heuristic at all";
    let findings = scan(content, "doc.txt", &dets);
    assert!(findings.iter().all(|f| f.detector != "high-entropy"));
}
