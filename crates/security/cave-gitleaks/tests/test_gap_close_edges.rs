// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge-case gap closure for cave-gitleaks public API.
//!
//! Focuses on failure modes, boundary conditions, state transitions, and
//! serde round-trips that the in-module unit tests do not exercise. All
//! tests go through the public re-exports in `lib.rs` (no `src/` reach-in).

use std::io::Write;

use cave_gitleaks::{
    Allowlist, Baseline, BaselineFile, Config, Decoder, DecoderChain, Detector, Finding,
    ProtectOutcome, builtin_rules, default_stopwords, detect_with_decoders, filter_with_stopwords,
    protect_staged_blobs, protect_staged_with, redact, write_csv, write_json, write_junit,
    write_sarif,
};

// ── helpers ────────────────────────────────────────────────────────────

fn mk_finding(rule_id: &str, file: &str, line: usize) -> Finding {
    let mut f = Finding {
        description: format!("desc-{rule_id}"),
        start_line: line,
        end_line: line,
        start_column: 1,
        end_column: 10,
        match_text: "AKIA****".into(),
        secret: "AKIA****".into(),
        file: file.into(),
        symlink_file: String::new(),
        commit: String::new(),
        entropy: 4.0,
        author: String::new(),
        email: String::new(),
        date: String::new(),
        message: String::new(),
        tags: vec![],
        rule_id: rule_id.into(),
        fingerprint: String::new(),
    };
    f.fingerprint = f.compute_fingerprint();
    f
}

// ── redact() edges ─────────────────────────────────────────────────────

#[test]
fn redact_exact_five_chars_keeps_prefix_and_one_star() {
    // len=5: prefix(4) + 1 star (saturating_sub then min(12)).
    let r = redact("ABCDE");
    assert_eq!(r, "ABCD*");
}

#[test]
fn redact_sixteen_chars_caps_at_twelve_stars() {
    // len=16: prefix(4) + min(12, 12) = 12 stars.
    let r = redact("AAAABBBBCCCCDDDD");
    assert_eq!(r.len(), 16);
    assert_eq!(r.matches('*').count(), 12);
}

#[test]
fn redact_handles_unicode_prefix_safely() {
    // Multi-byte codepoints; we count chars not bytes for the prefix.
    let r = redact("héllo-secret-extra");
    // First 4 chars: h é l l, then stars (15 - 4 = 11 stars capped by 12).
    assert!(r.starts_with("héll"));
    assert!(r[r.find('o').unwrap_or(r.len())..].chars().any(|c| c == '*') || r.contains('*'));
}

// ── Finding fingerprint state transitions ──────────────────────────────

#[test]
fn fingerprint_changes_when_commit_set_after_working_tree() {
    let mut f = mk_finding("aws-access-token", "a.rs", 7);
    let working_fp = f.compute_fingerprint();
    assert!(working_fp.starts_with("WORKING:"));
    f.commit = "deadbeef".into();
    let commit_fp = f.compute_fingerprint();
    assert!(commit_fp.starts_with("deadbeef:"));
    assert_ne!(working_fp, commit_fp);
}

#[test]
fn finding_serde_preserves_all_upstream_field_names() {
    let mut f = mk_finding("github-pat", "src/lib.rs", 3);
    f.tags = vec!["high".into(), "audit".into()];
    f.commit = "c0ffee".into();
    let j = serde_json::to_string(&f).unwrap();
    for tag in [
        "Description",
        "StartLine",
        "EndLine",
        "StartColumn",
        "EndColumn",
        "Match",
        "Secret",
        "File",
        "SymlinkFile",
        "Commit",
        "Entropy",
        "Author",
        "Email",
        "Date",
        "Message",
        "Tags",
        "RuleID",
        "Fingerprint",
    ] {
        assert!(j.contains(&format!("\"{tag}\"")), "missing tag {tag}");
    }
}

// ── builtin rule catalogue ────────────────────────────────────────────

#[test]
fn builtin_rules_all_have_keyword_prefilters_except_private_key_block() {
    // Every rule should declare at least one keyword (cheap pre-filter
    // is the whole MVP perf budget).
    for r in builtin_rules() {
        assert!(!r.keywords.is_empty(), "rule {} missing keywords", r.id);
    }
}

#[test]
fn builtin_jwt_rule_matches_three_part_token() {
    let jwt =
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    let any = builtin_rules()
        .into_iter()
        .find(|r| r.id == "jwt")
        .map(|r| r.regex.is_match(jwt))
        .unwrap_or(false);
    assert!(any);
}

#[test]
fn builtin_stripe_secret_matches_both_test_and_live_keys() {
    let r = builtin_rules()
        .into_iter()
        .find(|r| r.id == "stripe-secret-key")
        .unwrap();
    assert!(r.regex.is_match("sk_test_abcdefghijklmnopqrstuvwx"));
    assert!(r.regex.is_match("sk_live_abcdefghijklmnopqrstuvwx"));
    assert!(r.regex.is_match("rk_test_abcdefghijklmnopqrstuvwx"));
    assert!(!r.regex.is_match("sk_dev_abcdefghijklmnopqrstuvwx"));
}

// ── Detector edges ─────────────────────────────────────────────────────

#[test]
fn detector_empty_path_skips_global_path_allowlist_check() {
    // Stdin path-empty case: path_allowed short-circuits.
    let mut d = Detector::with_builtins();
    d.allowlist = Allowlist {
        description: String::new(),
        paths: vec![regex::Regex::new(".*").unwrap()],
        regexes: vec![],
        commits: vec![],
    };
    // Even with allow-everything path regex, empty path must still scan.
    let findings = d.scan_str("", "AKIAIOSFODNN7EXAMPLE");
    assert_eq!(findings.len(), 1);
}

#[test]
fn detector_redact_match_false_preserves_match_text() {
    let mut d = Detector::with_builtins();
    d.redact_match = false;
    let findings = d.scan_str("f.rs", "AKIAIOSFODNN7EXAMPLE");
    assert_eq!(findings.len(), 1);
    // Match text retains the raw value when redact_match=false.
    assert_eq!(findings[0].match_text, "AKIAIOSFODNN7EXAMPLE");
    // Secret is still redacted regardless.
    assert!(findings[0].secret.starts_with("AKIA"));
    assert!(findings[0].secret.contains('*'));
}

#[test]
fn detector_path_scoped_rule_skips_off_path_files() {
    let r = cave_gitleaks::Rule::new("scoped", "scoped", r"\bSECRET-[A-Z0-9]+\b")
        .unwrap()
        .with_keywords(["SECRET-"])
        .with_path(r"\.env$")
        .unwrap();
    let d = Detector::new(vec![r], Allowlist::default());
    assert!(d.scan_str("src/main.rs", "SECRET-ABC123").is_empty());
    assert_eq!(d.scan_str("config/.env", "SECRET-ABC123").len(), 1);
}

#[test]
fn detector_emits_one_finding_per_matched_line() {
    // AKIA prefix + exactly 16 [A-Z0-9] chars per upstream regex.
    let d = Detector::with_builtins();
    let blob = "AKIAIOSFODNN7EXAMPLE\nAKIAABCDEFGHIJKLMNOP\nclean line\nAKIAZZZZZZZZZZZZZZZZ\n";
    let findings = d.scan_str("blob.txt", blob);
    let aws: Vec<_> = findings
        .into_iter()
        .filter(|f| f.rule_id == "aws-access-token")
        .collect();
    assert_eq!(aws.len(), 3, "got {} findings", aws.len());
    assert_eq!(aws[0].start_line, 1);
    assert_eq!(aws[1].start_line, 2);
    assert_eq!(aws[2].start_line, 4);
}

#[test]
fn detector_fingerprint_present_on_every_emitted_finding() {
    let d = Detector::with_builtins();
    let findings = d.scan_str("path.txt", "AKIAIOSFODNN7EXAMPLE");
    assert_eq!(findings.len(), 1);
    assert!(!findings[0].fingerprint.is_empty());
    assert!(findings[0].fingerprint.contains("aws-access-token"));
}

// ── Decoder chain edges ───────────────────────────────────────────────

#[test]
fn decoder_base64_invalid_chars_return_none() {
    // The pure base64_decode_bytes path is via Decoder enum.
    assert!(Decoder::Base64.try_decode("!@#$%^&*()").is_none());
}

#[test]
fn decoder_gzip_rejects_too_short_input() {
    // Single byte → cannot satisfy the 2-byte magic check.
    assert!(Decoder::Gzip.try_decode_bytes(&[0x1f]).is_none());
    assert!(Decoder::Gzip.try_decode_bytes(&[]).is_none());
}

#[test]
fn decoder_chain_default_order_is_base64_then_gzip() {
    let c = DecoderChain::default_chain();
    assert_eq!(c.steps.len(), 2);
    assert!(matches!(c.steps[0], Decoder::Base64));
    assert!(matches!(c.steps[1], Decoder::Gzip));
}

#[test]
fn detect_with_decoders_finds_secret_inside_base64_blob() {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    // Encode a base64 wrapper containing a known AWS key directly.
    fn b64(input: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut i = 0;
        while i + 3 <= input.len() {
            let n = ((input[i] as u32) << 16)
                | ((input[i + 1] as u32) << 8)
                | (input[i + 2] as u32);
            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
            out.push(TABLE[(n & 0x3f) as usize] as char);
            i += 3;
        }
        let rem = input.len() - i;
        if rem == 1 {
            let n = (input[i] as u32) << 16;
            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push_str("==");
        } else if rem == 2 {
            let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        out
    }
    let secret_line = "AKIAIOSFODNN7EXAMPLE";
    let blob = b64(secret_line.as_bytes());
    let d = Detector::with_builtins();
    let chain = DecoderChain::default_chain();
    let findings = detect_with_decoders(&d, "wrap.txt", &blob, &chain, 1);
    assert!(
        findings.iter().any(|f| f.rule_id == "aws-access-token"),
        "decoder must surface base64-wrapped AWS key"
    );
    // Also ensure the helper writer compiles for parity (gzip side).
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(b"hi").unwrap();
    let _bytes = enc.finish().unwrap();
}

#[test]
fn detect_with_decoders_zero_depth_skips_decoding_pass() {
    // Zero-depth → only the surface scan runs, no decoded surfaces examined.
    let d = Detector::with_builtins();
    let chain = DecoderChain::default_chain();
    // A surface that has no AWS key but is base64 of one.
    let plain = "QUtJQUlPU0ZPRE5ON0VYQU1QTEU="; // b64("AKIAIOSFODNN7EXAMPLE")
    let with_depth = detect_with_decoders(&d, "x", plain, &chain, 1);
    let without = detect_with_decoders(&d, "x", plain, &chain, 0);
    // With depth, the decoded form surfaces the key; without, nothing.
    assert!(
        with_depth.iter().any(|f| f.rule_id == "aws-access-token"),
        "depth=1 should decode and find"
    );
    assert!(
        !without.iter().any(|f| f.rule_id == "aws-access-token"),
        "depth=0 should not decode"
    );
}

// ── Stopwords edges ────────────────────────────────────────────────────

#[test]
fn stopwords_default_pack_is_nonempty_and_lowercase() {
    let pack = default_stopwords();
    assert!(pack.len() >= 8);
    for w in &pack {
        // Author wrote them lowercase; the filter lowercases on each call
        // but normalisation up front saves a per-finding allocation.
        assert_eq!(w.as_str(), w.to_ascii_lowercase());
    }
}

#[test]
fn stopwords_case_insensitive_against_uppercase_secret() {
    // Secret is uppercase, stopword is lowercase; lowercase compare must match.
    let mut f = mk_finding("r", "f", 1);
    f.match_text = "API_KEY=PLACEHOLDER_TOKEN".into();
    f.secret = "PLACEHOLDER_TOKEN".into();
    let kept = filter_with_stopwords(vec![f], &["placeholder".into()]);
    assert!(kept.is_empty());
}

#[test]
fn stopwords_match_on_secret_field_alone() {
    // match_text has no stopword, secret field does → still dropped.
    let mut f = mk_finding("r", "f", 1);
    f.match_text = "clean".into();
    f.secret = "FAKE".into();
    let kept = filter_with_stopwords(vec![f], &["fake".into()]);
    assert!(kept.is_empty());
}

#[test]
fn stopwords_no_match_preserves_findings() {
    let f1 = mk_finding("r1", "f", 1);
    let f2 = mk_finding("r2", "f", 2);
    let kept = filter_with_stopwords(vec![f1, f2], &["noopneverappears".into()]);
    assert_eq!(kept.len(), 2);
}

// ── Baseline edges ────────────────────────────────────────────────────

#[test]
fn baseline_filter_strips_known_findings() {
    let f1 = mk_finding("aws-access-token", "a.rs", 1);
    let f2 = mk_finding("github-pat", "a.rs", 5);
    let mut b = Baseline::default();
    b.ingest(f1.fingerprint.clone());
    let remaining = b.filter(vec![f1.clone(), f2.clone()]);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].rule_id, "github-pat");
}

#[test]
fn baseline_from_json_malformed_returns_error() {
    let err = Baseline::from_json("{not json").unwrap_err();
    let s = format!("{err}");
    assert!(s.contains("JSON") || s.contains("json") || s.contains("expected"));
}

#[test]
fn baseline_file_parse_empty_yields_default() {
    let b = BaselineFile::parse("").unwrap();
    assert!(b.entries.is_empty());
    let bl: Baseline = b.into();
    assert!(bl.is_empty());
}

#[test]
fn baseline_dedup_is_set_semantic() {
    // Two findings with identical fingerprint collapse in the underlying set.
    let json = r#"[{"Fingerprint":"x"},{"Fingerprint":"x"},{"Fingerprint":"y"}]"#;
    let b = Baseline::from_json(json).unwrap();
    assert_eq!(b.len(), 2);
}

// ── Protect command edges ──────────────────────────────────────────────

#[test]
fn protect_clean_when_blobs_have_no_secrets() {
    let staged = vec![
        ("README.md".into(), "no secrets here".into()),
        ("src/lib.rs".into(), "// also clean".into()),
    ];
    assert!(protect_staged_blobs(&staged).is_clean());
}

#[test]
fn protect_blocks_when_any_blob_leaks() {
    let staged = vec![
        ("ok.rs".into(), "clean".into()),
        ("leak.rs".into(), "AKIAIOSFODNN7EXAMPLE".into()),
    ];
    let outcome = protect_staged_blobs(&staged);
    match outcome {
        ProtectOutcome::Blocked { findings } => {
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].file, "leak.rs");
        }
        ProtectOutcome::Clean => panic!("expected blocked"),
    }
}

#[test]
fn protect_with_custom_detector_uses_supplied_rules_only() {
    // Build a detector with a single dummy rule that never matches.
    let r = cave_gitleaks::Rule::new("noop", "no match ever", "ZZZZ_IMPOSSIBLE_TOKEN").unwrap();
    let d = Detector::new(vec![r], Allowlist::default());
    let staged = vec![("a.rs".into(), "AKIAIOSFODNN7EXAMPLE".into())];
    // Builtin AWS rule is absent so even a real AKIA key is clean here.
    assert!(protect_staged_with(&d, &staged).is_clean());
}

// ── Reporters: CSV / JUnit / SARIF edges ──────────────────────────────

#[test]
fn write_csv_quotes_fields_containing_commas() {
    let mut f = mk_finding("rule,with,commas", "file,a.rs", 1);
    f.description = "desc, with comma".into();
    let mut buf = Vec::new();
    write_csv(&mut buf, &[f]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    // The header row exists.
    assert!(s.starts_with("RuleID,Commit,File,"));
    // Field with comma must be quoted.
    assert!(s.contains("\"rule,with,commas\""));
    assert!(s.contains("\"file,a.rs\""));
    assert!(s.contains("\"desc, with comma\""));
}

#[test]
fn write_csv_doubles_embedded_quotes_per_rfc_4180() {
    let mut f = mk_finding("r", "f", 1);
    f.match_text = "has \"quote\" inside".into();
    let mut buf = Vec::new();
    write_csv(&mut buf, &[f]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("\"has \"\"quote\"\" inside\""));
}

#[test]
fn write_csv_joins_tags_with_pipe_delimiter() {
    let mut f = mk_finding("r", "f", 1);
    f.tags = vec!["a".into(), "b".into(), "c".into()];
    let mut buf = Vec::new();
    write_csv(&mut buf, &[f]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("a|b|c"));
}

#[test]
fn write_csv_empty_findings_emits_header_only() {
    let mut buf = Vec::new();
    write_csv(&mut buf, &[]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    // Single line — the header — terminated with newline.
    assert_eq!(s.lines().count(), 1);
    assert!(s.starts_with("RuleID,"));
}

#[test]
fn write_junit_xml_escapes_special_characters() {
    let mut f = mk_finding("rule<a>&b", "<file>.rs", 1);
    f.description = "ampersand & bracket > test".into();
    let mut buf = Vec::new();
    write_junit(&mut buf, &[f]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("&lt;"));
    assert!(s.contains("&gt;"));
    assert!(s.contains("&amp;"));
    // No raw special chars survive in the escaped form (for these specific markers).
    assert!(!s.contains("rule<a>"));
}

#[test]
fn write_junit_envelope_has_testsuite_tests_and_failures_counts() {
    let f1 = mk_finding("r1", "f.rs", 1);
    let f2 = mk_finding("r2", "g.rs", 2);
    let mut buf = Vec::new();
    write_junit(&mut buf, &[f1, f2]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.starts_with("<?xml"));
    assert!(s.contains("tests=\"2\""));
    assert!(s.contains("failures=\"2\""));
    assert!(s.contains("</testsuite>"));
}

#[test]
fn write_junit_empty_findings_produces_zero_counts() {
    let mut buf = Vec::new();
    write_junit(&mut buf, &[]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    assert!(s.contains("tests=\"0\""));
    assert!(s.contains("failures=\"0\""));
}

#[test]
fn write_json_round_trips_serde_finding_array() {
    let f = mk_finding("aws-access-token", "src/main.rs", 12);
    let mut buf = Vec::new();
    write_json(&mut buf, &[f.clone()]).unwrap();
    let s = String::from_utf8(buf).unwrap();
    // Re-parse as JSON value and assert the upstream keys appear.
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["RuleID"], "aws-access-token");
    assert_eq!(arr[0]["StartLine"], 12);
    assert_eq!(arr[0]["File"], "src/main.rs");
}

#[test]
fn write_sarif_results_count_matches_findings_count() {
    let mut findings = Vec::new();
    for i in 1..=4 {
        findings.push(mk_finding("aws-access-token", "x.rs", i));
    }
    let mut buf = Vec::new();
    write_sarif(&mut buf, &findings).unwrap();
    let s = String::from_utf8(buf).unwrap();
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let results = v["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 4);
    // But the rule descriptor stays single because all share an id.
    let rules = v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
}

// ── Config + extend ─────────────────────────────────────────────────────

#[test]
fn config_extend_use_default_includes_builtin_pack() {
    let toml_text = r#"
[extend]
useDefault = true

[[rules]]
id          = "custom-rule"
description = "custom"
regex       = "CUSTOM-[A-Z]+"
keywords    = ["CUSTOM-"]
"#;
    let cfg = Config::parse(toml_text).unwrap();
    let (rules, _) = cfg.into_rules_with_extend().unwrap();
    let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"custom-rule"));
    assert!(ids.contains(&"aws-access-token"));
}

#[test]
fn config_extend_disabled_rules_strips_builtin() {
    let toml_text = r#"
[extend]
useDefault    = true
disabledRules = ["aws-access-token"]
"#;
    let cfg = Config::parse(toml_text).unwrap();
    let (rules, _) = cfg.into_rules_with_extend().unwrap();
    let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
    assert!(!ids.contains(&"aws-access-token"));
    // Some other built-in still present.
    assert!(ids.contains(&"github-pat"));
}

#[test]
fn config_extend_user_rule_overrides_builtin_by_id() {
    let toml_text = r#"
[extend]
useDefault = true

[[rules]]
id          = "aws-access-token"
description = "custom AWS"
regex       = "MY_CUSTOM_AWS"
keywords    = ["MY_CUSTOM_AWS"]
"#;
    let cfg = Config::parse(toml_text).unwrap();
    let (rules, _) = cfg.into_rules_with_extend().unwrap();
    let aws = rules
        .iter()
        .find(|r| r.id == "aws-access-token")
        .expect("aws rule still present");
    assert_eq!(aws.description, "custom AWS");
    assert!(aws.regex.is_match("MY_CUSTOM_AWS"));
    // Confirm only one copy survived after override.
    let count = rules.iter().filter(|r| r.id == "aws-access-token").count();
    assert_eq!(count, 1);
}

#[test]
fn config_extend_no_default_returns_only_user_rules() {
    let toml_text = r#"
[[rules]]
id    = "only-this"
regex = "ONLY"
"#;
    let cfg = Config::parse(toml_text).unwrap();
    let (rules, _) = cfg.into_rules_with_extend().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].id, "only-this");
}
