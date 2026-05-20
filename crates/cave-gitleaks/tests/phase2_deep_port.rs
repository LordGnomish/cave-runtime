// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-gitleaks Phase 2 deep-port — protect, baseline, CSV/JUnit
//! reporters, decoders (base64 + gzip), stopwords, Extend/UseDefault
//! resolver.

use cave_gitleaks::baseline::{Baseline, BaselineFile};
use cave_gitleaks::config::Config;
use cave_gitleaks::decoders::{detect_with_decoders, DecoderChain};
use cave_gitleaks::finding::Finding;
use cave_gitleaks::protect::{protect_staged_blobs, ProtectOutcome};
use cave_gitleaks::report::{write_csv, write_junit};
use cave_gitleaks::stopwords::filter_with_stopwords;
use cave_gitleaks::Detector;

fn finding(rule: &str, file: &str, line: usize) -> Finding {
    Finding {
        description: format!("desc-{rule}"),
        start_line: line,
        end_line: line,
        start_column: 1,
        end_column: 10,
        match_text: "redacted".into(),
        secret: "redacted".into(),
        file: file.into(),
        symlink_file: String::new(),
        commit: String::new(),
        entropy: 0.0,
        author: String::new(),
        email: String::new(),
        date: String::new(),
        message: String::new(),
        tags: vec![],
        rule_id: rule.into(),
        fingerprint: format!("WORKING:{file}:{rule}:{line}"),
    }
}

// ─── CSV reporter ───────────────────────────────────────────────────────────

#[test]
fn csv_reporter_emits_header_then_rows() {
    let f1 = finding("aws", "src/main.rs", 1);
    let f2 = finding("github-pat", "src/lib.rs", 5);
    let mut buf = Vec::new();
    write_csv(&mut buf, &[f1, f2]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    let lines: Vec<&str> = s.lines().collect();
    assert!(lines[0].contains("RuleID"));
    assert!(lines[0].contains("File"));
    assert!(lines[0].contains("StartLine"));
    assert_eq!(lines.len(), 3); // header + 2 rows
    assert!(lines[1].contains("aws"));
    assert!(lines[2].contains("github-pat"));
}

#[test]
fn csv_reporter_escapes_commas_and_quotes() {
    let mut f = finding("r", "path,with,commas.rs", 1);
    f.description = "has \"quotes\" inside".into();
    let mut buf = Vec::new();
    write_csv(&mut buf, &[f]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("\"path,with,commas.rs\""));
    assert!(s.contains("\"\"quotes\"\""));
}

// ─── JUnit reporter ─────────────────────────────────────────────────────────

#[test]
fn junit_reporter_emits_valid_xml_envelope() {
    let f = finding("aws", "src/main.rs", 12);
    let mut buf = Vec::new();
    write_junit(&mut buf, &[f]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.starts_with("<?xml"));
    assert!(s.contains("<testsuite"));
    assert!(s.contains("<testcase"));
    assert!(s.contains("<failure"));
    assert!(s.contains("aws"));
    assert!(s.contains("src/main.rs"));
}

#[test]
fn junit_empty_findings_emits_zero_failures() {
    let mut buf = Vec::new();
    write_junit(&mut buf, &[]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("failures=\"0\""));
}

// ─── Decoders ───────────────────────────────────────────────────────────────

#[test]
fn detect_finds_secret_in_base64_payload() {
    let d = Detector::with_builtins();
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let b64 = base64_encode(secret.as_bytes());
    let content = format!("payload = \"{}\"", b64);
    let chain = DecoderChain::default_chain();
    let findings = detect_with_decoders(&d, "src/x.rs", &content, &chain, 2);
    assert!(
        findings.iter().any(|f| f.rule_id == "aws-access-token"),
        "expected aws-access-token via base64 decode"
    );
}

#[test]
fn detect_respects_max_decode_depth() {
    // Triple-wrap an AWS key in base64; with depth=1 the secret should NOT
    // be visible.
    let d = Detector::with_builtins();
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let once = base64_encode(secret.as_bytes());
    let twice = base64_encode(once.as_bytes());
    let chain = DecoderChain::default_chain();
    let findings = detect_with_decoders(&d, "src/x.rs", &twice, &chain, 1);
    // Single-decode reveals `once` — still base64 of secret — so no AWS rule fires.
    assert!(findings.iter().all(|f| f.rule_id != "aws-access-token"));
}

#[test]
fn detect_finds_secret_in_gzip_payload() {
    let d = Detector::with_builtins();
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let gzipped = gzip_compress(secret.as_bytes());
    let b64_gz = base64_encode(&gzipped);
    let chain = DecoderChain::default_chain();
    let findings = detect_with_decoders(&d, "src/x.rs", &b64_gz, &chain, 3);
    assert!(findings.iter().any(|f| f.rule_id == "aws-access-token"));
}

// ─── Stopwords ──────────────────────────────────────────────────────────────

#[test]
fn stopwords_filter_drops_findings_whose_match_contains_stopword() {
    let f1 = finding("generic-api-key", "src/x.rs", 1);
    let mut f2 = finding("generic-api-key", "src/x.rs", 2);
    f2.match_text = "api_key = test123".into();
    let stopwords = vec!["test123".to_string()];
    let kept = filter_with_stopwords(vec![f1.clone(), f2.clone()], &stopwords);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].start_line, 1);
}

#[test]
fn stopwords_filter_case_insensitive() {
    let mut f = finding("r", "x", 1);
    f.match_text = "EXAMPLE-secret".into();
    let stopwords = vec!["example".to_string()];
    let kept = filter_with_stopwords(vec![f], &stopwords);
    assert!(kept.is_empty());
}

// ─── Baseline ───────────────────────────────────────────────────────────────

#[test]
fn baseline_loads_known_fingerprints_and_filters() {
    let baseline_toml = r#"
        [[entries]]
        fingerprint = "WORKING:src/main.rs:aws-access-token:1"
        rule_id     = "aws-access-token"
        file        = "src/main.rs"
        start_line  = 1
        note        = "known false positive — test fixture"
    "#;
    let baseline = BaselineFile::parse(baseline_toml).unwrap();
    let known = finding("aws-access-token", "src/main.rs", 1);
    let novel = finding("aws-access-token", "src/main.rs", 99);
    let kept = Baseline::from(baseline).filter(vec![known.clone(), novel.clone()]);
    assert_eq!(kept.len(), 1, "baselined finding suppressed");
    assert_eq!(kept[0].start_line, 99);
}

#[test]
fn baseline_handles_empty_file() {
    let baseline = BaselineFile::parse("").unwrap();
    let b = Baseline::from(baseline);
    let f = finding("r", "x", 1);
    let kept = b.filter(vec![f.clone()]);
    assert_eq!(kept.len(), 1);
}

// ─── Protect (pre-commit / pre-push staged-blob enforcement) ────────────────

#[test]
fn protect_returns_clean_when_no_secrets_in_staged_text() {
    let outcome = protect_staged_blobs(&[(String::from("src/main.rs"), String::from("println!()"))]);
    assert!(matches!(outcome, ProtectOutcome::Clean));
}

#[test]
fn protect_returns_blocked_with_findings_for_staged_secret() {
    let staged = vec![(
        "src/x.rs".to_string(),
        "let aws = \"AKIAIOSFODNN7EXAMPLE\";".to_string(),
    )];
    let outcome = protect_staged_blobs(&staged);
    match outcome {
        ProtectOutcome::Blocked { findings } => {
            assert!(findings.iter().any(|f| f.rule_id == "aws-access-token"));
        }
        ProtectOutcome::Clean => panic!("staged blob contains a leak — should block"),
    }
}

// ─── Extend / UseDefault config composition ─────────────────────────────────

#[test]
fn extend_use_default_pulls_builtins_into_user_config() {
    let user_toml = r#"
        [extend]
        useDefault = true

        [[rules]]
        id    = "user-custom"
        regex = "USER_TOKEN_[A-Z0-9]{6}"
    "#;
    let cfg = Config::parse(user_toml).expect("parse");
    let (rules, _) = cfg.into_rules_with_extend().expect("compile");
    // Should include built-ins + the user rule.
    assert!(rules.iter().any(|r| r.id == "aws-access-token"));
    assert!(rules.iter().any(|r| r.id == "user-custom"));
}

#[test]
fn extend_without_use_default_omits_builtins() {
    let user_toml = r#"
        [[rules]]
        id    = "user-only"
        regex = "MY_[A-Z]{3}"
    "#;
    let cfg = Config::parse(user_toml).expect("parse");
    let (rules, _) = cfg.into_rules_with_extend().expect("compile");
    assert!(rules.iter().all(|r| r.id != "aws-access-token"));
    assert!(rules.iter().any(|r| r.id == "user-only"));
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push(TABLE[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push_str("==");
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

fn gzip_compress(input: &[u8]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(input).unwrap();
    e.finish().unwrap()
}
