// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gap-close edge tests for cave-secrets.
//!
//! Focus: regex boundary cases, scan-state transitions, redaction
//! boundaries, false-positive filters, severity invariants, serde
//! round-trips for model types, and SecretsState construction.

use cave_secrets::detector::{Finding, Severity, builtin_detectors, scan, shannon_entropy};
use cave_secrets::models::{
    AllowlistEntry, Confidence, ScanRequest as ModelScanRequest, ScanResult, ScanStats,
    SecretFinding, SecretRule, SecretType, Severity as ModelSeverity,
};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Detector-side: regex boundary & false-positive filters
// ---------------------------------------------------------------------------

#[test]
fn aws_key_short_by_one_does_not_match() {
    // AKIA + 15 alnum is one short of the required 16 — should not fire.
    let dets = builtin_detectors();
    let content = "K=AKIA0123456789ABCDE"; // AKIA + 15 chars
    let findings = scan(content, "x.env", &dets);
    assert!(
        findings.iter().all(|f| f.detector != "aws-access-key"),
        "AKIA + 15 alnum must not match the AWS access-key pattern"
    );
}

#[test]
fn github_token_too_short_does_not_match() {
    // ghp_ followed by < 36 chars should not match.
    let dets = builtin_detectors();
    let content = "TOK=ghp_short";
    let findings = scan(content, "x.env", &dets);
    assert!(findings.iter().all(|f| f.detector != "github-token"));
}

#[test]
fn jwt_with_only_two_segments_does_not_match() {
    // JWT pattern requires three dot-separated parts.
    let dets = builtin_detectors();
    let content = "Bearer eyJabcdefghij.eyJabcdefghij";
    let findings = scan(content, "r.txt", &dets);
    assert!(findings.iter().all(|f| f.detector != "jwt-token"));
}

#[test]
fn slack_webhook_wrong_host_does_not_match() {
    // Pattern is anchored on hooks.slack.com/services/...
    let dets = builtin_detectors();
    let content = "URL=https://hooks.example.com/services/T01ABCDE/B01ABCDE/abc123";
    let findings = scan(content, "x.env", &dets);
    assert!(findings.iter().all(|f| f.detector != "slack-webhook"));
}

#[test]
fn password_assignment_short_value_does_not_match() {
    // Pattern requires at least 8 chars between quotes.
    let dets = builtin_detectors();
    let content = r#"password = "short""#; // 5 chars between quotes
    let findings = scan(content, "x.toml", &dets);
    assert!(
        findings
            .iter()
            .all(|f| f.detector != "password-assignment")
    );
}

#[test]
fn generic_api_key_requires_assignment_operator() {
    // "api_key something" without =/: must not match.
    let dets = builtin_detectors();
    let content = "Documentation about api_key usage in projects";
    let findings = scan(content, "doc.md", &dets);
    assert!(findings.iter().all(|f| f.detector != "generic-api-key"));
}

#[test]
fn private_key_plain_no_label_matches() {
    // The pattern allows the "(RSA |EC |DSA )?" group to be absent.
    let dets = builtin_detectors();
    let content = "-----BEGIN PRIVATE KEY-----\n";
    let findings = scan(content, "id_plain", &dets);
    assert!(findings.iter().any(|f| f.detector == "private-key"));
}

// ---------------------------------------------------------------------------
// Scan-state transitions: line numbering, dedup-by-coordinate, multi-line
// ---------------------------------------------------------------------------

#[test]
fn scan_assigns_increasing_line_numbers() {
    let dets = builtin_detectors();
    let content =
        "junk\nAKIAIOSFODNN7EXAMPLE\nmore_junk\nAKIAIOSFODNN7OTHERKEY\n";
    let findings: Vec<&Finding> = scan(content, "f.env", &dets)
        .into_iter()
        .filter(|f| f.detector == "aws-access-key")
        .collect::<Vec<_>>()
        .leak() // safe: bench-test scope only, but avoid lifetime issues
        .iter()
        .collect();
    // Two findings expected; lines must be strictly increasing.
    assert_eq!(findings.len(), 2);
    assert!(findings[0].line < findings[1].line);
    assert_eq!(findings[0].line, 2);
    assert_eq!(findings[1].line, 4);
}

#[test]
fn scan_same_line_multiple_detectors_each_produces_finding() {
    // A line carrying both an AWS key AND a github token must produce
    // one finding per matching detector — not collapse to a single hit.
    let dets = builtin_detectors();
    let content =
        "AKIAIOSFODNN7EXAMPLE ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef1234";
    let findings = scan(content, "x.env", &dets);
    assert!(findings.iter().any(|f| f.detector == "aws-access-key"));
    assert!(findings.iter().any(|f| f.detector == "github-token"));
}

#[test]
fn scan_empty_lines_do_not_panic() {
    let dets = builtin_detectors();
    let content = "\n\n\n\n";
    let findings = scan(content, "blank.txt", &dets);
    assert!(findings.is_empty());
}

#[test]
fn scan_trailing_newline_does_not_create_phantom_line() {
    // "x\n" has exactly one line per str::lines().
    let dets = builtin_detectors();
    let content = "AKIAIOSFODNN7EXAMPLE\n";
    let findings = scan(content, "x.env", &dets);
    let aws = findings
        .iter()
        .find(|f| f.detector == "aws-access-key")
        .expect("aws finding");
    assert_eq!(aws.line, 1);
}

// ---------------------------------------------------------------------------
// Redaction boundary
// ---------------------------------------------------------------------------

#[test]
fn redact_match_threshold_at_21_chars_redacts() {
    // detector::redact_match only redacts when line.len() > 20.
    // Construct a line of exactly 21 chars that contains a matching AWS key.
    // "AKIAIOSFODNN7EXAMPLE" is 20 chars, so " AKIAIOSFODNN7EXAMPLE" is 21.
    let dets = builtin_detectors();
    let content = " AKIAIOSFODNN7EXAMPLE"; // 21 chars
    let findings = scan(content, "x.env", &dets);
    let aws = findings
        .iter()
        .find(|f| f.detector == "aws-access-key")
        .unwrap();
    assert!(
        aws.matched.contains("..."),
        "21-char line should be redacted with ellipsis"
    );
}

#[test]
fn redact_match_exactly_20_chars_not_redacted() {
    // Exactly 20 chars matches but should not be redacted.
    let dets = builtin_detectors();
    let content = "AKIAIOSFODNN7EXAMPLE"; // exactly 20 chars
    let findings = scan(content, "x.env", &dets);
    let aws = findings
        .iter()
        .find(|f| f.detector == "aws-access-key")
        .unwrap();
    assert!(!aws.matched.contains("..."));
    assert_eq!(aws.matched, "AKIAIOSFODNN7EXAMPLE");
}

// ---------------------------------------------------------------------------
// Entropy heuristic — boundary & filter
// ---------------------------------------------------------------------------

#[test]
fn entropy_line_exactly_20_chars_does_not_fire() {
    // The high-entropy detector requires line.len() > 20.
    let dets = builtin_detectors();
    // 20-char line with hint keyword "key" but at length boundary.
    let content = "key=aBcDeFgHiJkLmNoP"; // 20 chars exactly
    let findings = scan(content, "x.txt", &dets);
    assert!(findings.iter().all(|f| f.detector != "high-entropy"));
}

#[test]
fn entropy_uppercase_hint_keyword_fires() {
    // "KEY" or "SECRET" uppercase hints should also fire entropy detector.
    let dets = builtin_detectors();
    let content = "PUBLIC_KEY=aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789";
    let findings = scan(content, "x.env", &dets);
    assert!(findings.iter().any(|f| f.detector == "high-entropy"));
}

#[test]
fn entropy_low_value_with_hint_does_not_fire() {
    // Has "key" hint but entropy too low (repeated chars).
    let dets = builtin_detectors();
    let content = "key=aaaaaaaaaaaaaaaaaaaaaaaaaaaa"; // long but low entropy
    let findings = scan(content, "x.env", &dets);
    assert!(findings.iter().all(|f| f.detector != "high-entropy"));
}

// ---------------------------------------------------------------------------
// Severity & verification invariants
// ---------------------------------------------------------------------------

#[test]
fn private_key_severity_is_critical() {
    let dets = builtin_detectors();
    let findings = scan(
        "-----BEGIN RSA PRIVATE KEY-----",
        "id_rsa",
        &dets,
    );
    let pk = findings
        .iter()
        .find(|f| f.detector == "private-key")
        .unwrap();
    assert_eq!(pk.severity, Severity::Critical);
}

#[test]
fn azure_connection_string_severity_is_critical() {
    let dets = builtin_detectors();
    let content = "AZ=DefaultEndpointsProtocol=https;AccountName=mystore;AccountKey=YWJjZGVmZ2hpamtsbW5vcA==";
    let findings = scan(content, "az.env", &dets);
    let az = findings
        .iter()
        .find(|f| f.detector == "azure-connection-string")
        .unwrap();
    assert_eq!(az.severity, Severity::Critical);
}

#[test]
fn all_findings_unverified_after_scan() {
    // The scan() function does not perform verification — `verified` must
    // always be false at this stage. Verification is a separate pipeline step.
    let dets = builtin_detectors();
    let content =
        "AKIAIOSFODNN7EXAMPLE\nghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef1234\n-----BEGIN RSA PRIVATE KEY-----";
    let findings = scan(content, "mixed.txt", &dets);
    assert!(!findings.is_empty());
    assert!(findings.iter().all(|f| !f.verified));
}

#[test]
fn detector_verify_flag_propagated_in_builtins() {
    // aws/github/slack are flagged verify=true; private-key/jwt are not.
    let dets = builtin_detectors();
    let aws = dets.iter().find(|d| d.name == "aws-access-key").unwrap();
    assert!(aws.verify);
    let pk = dets.iter().find(|d| d.name == "private-key").unwrap();
    assert!(!pk.verify);
    let jwt = dets.iter().find(|d| d.name == "jwt-token").unwrap();
    assert!(!jwt.verify);
}

// ---------------------------------------------------------------------------
// Shannon entropy — mathematical properties
// ---------------------------------------------------------------------------

#[test]
fn shannon_entropy_single_char_is_zero() {
    // -1 * log2(1) = 0
    assert_eq!(shannon_entropy("x"), 0.0);
}

#[test]
fn shannon_entropy_is_monotone_under_uniformity() {
    // Uniform distribution over k symbols has entropy log2(k).
    // 4 distinct equally-frequent chars => entropy ≈ 2.0
    let e = shannon_entropy("abcdabcdabcdabcd");
    assert!((e - 2.0).abs() < 0.01, "expected ~2.0, got {e}");
}

#[test]
fn shannon_entropy_increases_with_diversity() {
    let low = shannon_entropy("aaaaaaaaaaaaaaaa");
    let mid = shannon_entropy("aaaaaaaabbbbbbbb");
    let hi = shannon_entropy("abcdefghijklmnop");
    assert!(low < mid && mid < hi);
}

// ---------------------------------------------------------------------------
// Models — serde round-trip & rename_all snake_case
// ---------------------------------------------------------------------------

#[test]
fn secret_rule_serde_roundtrip() {
    let r = SecretRule {
        id: "aws-1".to_string(),
        name: "AWS Access".to_string(),
        secret_type: SecretType::AwsCredential,
        pattern: r"AKIA[0-9A-Z]{16}".to_string(),
        keywords: vec!["AKIA".to_string(), "aws".to_string()],
        confidence: Confidence::High,
        entropy_threshold: 3.5,
        severity: ModelSeverity::Critical,
    };
    let json = serde_json::to_string(&r).expect("serialize");
    let back: SecretRule = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.id, r.id);
    assert_eq!(back.secret_type, r.secret_type);
    assert_eq!(back.confidence, r.confidence);
    assert_eq!(back.severity, r.severity);
    assert_eq!(back.keywords, r.keywords);
    assert!((back.entropy_threshold - r.entropy_threshold).abs() < f64::EPSILON);
}

#[test]
fn secret_finding_serde_roundtrip_with_optionals() {
    let f = SecretFinding {
        id: "id-1".to_string(),
        rule_id: "gh-1".to_string(),
        rule_name: "GitHub Token".to_string(),
        secret_type: SecretType::GithubToken,
        file_path: "src/x.rs".to_string(),
        line_number: None,
        column: None,
        redacted_value: "ghp_****".to_string(),
        entropy: 4.2,
        confidence: Confidence::Medium,
        context: "let t = \"ghp_****\";".to_string(),
        commit: Some("deadbeef".to_string()),
    };
    let json = serde_json::to_string(&f).unwrap();
    let back: SecretFinding = serde_json::from_str(&json).unwrap();
    assert_eq!(back.line_number, None);
    assert_eq!(back.commit.as_deref(), Some("deadbeef"));
    assert_eq!(back.secret_type, SecretType::GithubToken);
}

#[test]
fn scan_result_serde_with_findings() {
    let f = SecretFinding {
        id: "id-2".to_string(),
        rule_id: "r".to_string(),
        rule_name: "n".to_string(),
        secret_type: SecretType::StripeKey,
        file_path: "a".to_string(),
        line_number: Some(7),
        column: Some(3),
        redacted_value: "sk_****".to_string(),
        entropy: 5.0,
        confidence: Confidence::Low,
        context: "x".to_string(),
        commit: None,
    };
    let r = ScanResult {
        file_path: "a".to_string(),
        findings: vec![f],
        scanned_lines: 42,
        scanned_bytes: 1000,
        duration_ms: 12,
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: ScanResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.findings.len(), 1);
    assert_eq!(back.findings[0].line_number, Some(7));
    assert_eq!(back.scanned_lines, 42);
}

#[test]
fn scan_stats_serde_with_populated_maps() {
    let mut by_type = HashMap::new();
    by_type.insert("aws_credential".to_string(), 3usize);
    by_type.insert("github_token".to_string(), 2usize);
    let mut by_sev = HashMap::new();
    by_sev.insert("critical".to_string(), 4usize);
    by_sev.insert("high".to_string(), 1usize);
    let s = ScanStats {
        total_findings: 5,
        by_type,
        by_severity: by_sev,
        files_scanned: 10,
        high_entropy_count: 2,
        high_confidence_count: 4,
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: ScanStats = serde_json::from_str(&json).unwrap();
    assert_eq!(back.total_findings, 5);
    assert_eq!(back.by_type.get("aws_credential"), Some(&3));
    assert_eq!(back.by_severity.get("critical"), Some(&4));
}

#[test]
fn allowlist_entry_roundtrip() {
    let e = AllowlistEntry {
        id: "al-1".to_string(),
        pattern: "tests/fixtures/**".to_string(),
        reason: "fixture data, not real secrets".to_string(),
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: AllowlistEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, "al-1");
    assert_eq!(back.pattern, "tests/fixtures/**");
}

#[test]
fn scan_request_redact_round_trips_true() {
    let req = ModelScanRequest {
        content: "x".to_string(),
        file_path: "a".to_string(),
        redact: true,
    };
    let json = serde_json::to_string(&req).unwrap();
    let back: ModelScanRequest = serde_json::from_str(&json).unwrap();
    assert!(back.redact);
}

#[test]
fn confidence_severity_total_ordering_is_consistent() {
    // All cross-pair orderings.
    let cs = [Confidence::Low, Confidence::Medium, Confidence::High];
    for (i, a) in cs.iter().enumerate() {
        for (j, b) in cs.iter().enumerate() {
            assert_eq!(a.cmp(b), i.cmp(&j));
        }
    }
    let svs = [
        ModelSeverity::Low,
        ModelSeverity::Medium,
        ModelSeverity::High,
        ModelSeverity::Critical,
    ];
    for (i, a) in svs.iter().enumerate() {
        for (j, b) in svs.iter().enumerate() {
            assert_eq!(a.cmp(b), i.cmp(&j));
        }
    }
}

#[test]
fn secret_type_as_str_distinct_for_all_variants() {
    // No two variants may collide on as_str().
    let variants = [
        SecretType::ApiKey,
        SecretType::AwsCredential,
        SecretType::GithubToken,
        SecretType::GitlabToken,
        SecretType::SlackToken,
        SecretType::PrivateKey,
        SecretType::Certificate,
        SecretType::Password,
        SecretType::DatabaseUrl,
        SecretType::GenericSecret,
        SecretType::GoogleApiKey,
        SecretType::StripeKey,
        SecretType::SendgridKey,
        SecretType::JwtSecret,
    ];
    let mut seen = std::collections::HashSet::new();
    for v in &variants {
        assert!(
            seen.insert(v.as_str()),
            "duplicate as_str() for variant {v:?}"
        );
    }
    assert_eq!(seen.len(), variants.len());
}

// ---------------------------------------------------------------------------
// SecretsState & module constant
// ---------------------------------------------------------------------------

#[test]
fn secrets_state_default_has_expected_minimum_detectors() {
    let st = cave_secrets::SecretsState::default();
    // Builtin set must include the core/high-impact detectors.
    let names: Vec<&str> = st.detectors.iter().map(|d| d.name).collect();
    for required in [
        "aws-access-key",
        "github-token",
        "private-key",
        "jwt-token",
        "slack-webhook",
        "azure-connection-string",
        "password-assignment",
        "generic-api-key",
    ] {
        assert!(names.contains(&required), "missing detector: {required}");
    }
}

#[test]
fn module_name_constant_stable() {
    assert_eq!(cave_secrets::MODULE_NAME, "secrets");
}
