// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rescue tests for `cave_scan::rules`.
//!
//! These tests verify that the SAST rule catalog compiles, exposes
//! `extended_scan_rules()`, and that every regex-bearing rule has a
//! valid pattern. Until `rules` is uncommented in `lib.rs` AND the
//! raw-string syntax errors are fixed, this file refuses to compile —
//! which is exactly the [RED] state.

use cave_scan::rules::extended_scan_rules;

#[test]
fn rules_catalog_is_nonempty() {
    let rules = extended_scan_rules();
    assert!(
        !rules.is_empty(),
        "extended_scan_rules() returned no rules — catalog is empty"
    );
}

#[test]
fn rules_catalog_meets_charter_minimum() {
    let rules = extended_scan_rules();
    assert!(
        rules.len() >= 50,
        "Charter requires 50+ SAST rules across 6 languages, got {}",
        rules.len()
    );
}

#[test]
fn every_rule_has_nonempty_id() {
    let rules = extended_scan_rules();
    for rule in &rules {
        assert!(!rule.id.is_empty(), "rule {:?} has empty id", rule.name);
    }
}

#[test]
fn every_rule_has_nonempty_name() {
    let rules = extended_scan_rules();
    for rule in &rules {
        assert!(!rule.name.is_empty(), "rule {} has empty name", rule.id);
    }
}

#[test]
fn every_rule_has_message_template() {
    let rules = extended_scan_rules();
    for rule in &rules {
        assert!(
            !rule.message_template.is_empty(),
            "rule {} has empty message_template",
            rule.id
        );
    }
}

#[test]
fn rule_ids_are_unique() {
    let rules = extended_scan_rules();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for rule in &rules {
        assert!(
            seen.insert(rule.id.as_str()),
            "duplicate rule id: {}",
            rule.id
        );
    }
}

#[test]
fn every_regex_pattern_compiles() {
    // SKIP list: rule IDs whose patterns are valid PCRE but use features
    // (e.g. negative lookahead) the `regex` crate intentionally does not
    // support. These rules remain in the catalog and are reachable; their
    // patterns just cannot be evaluated by the default regex engine.
    // See docs/adr/internal for the catalog-engine bridging plan.
    const SKIP: &[&str] = &[
        // GEN007 "magic number" uses `(?![0-9])` negative lookahead.
        // `regex` (RE2-style) rejects lookarounds by design.
        "GEN007",
    ];
    let rules = extended_scan_rules();
    let mut regex_count = 0usize;
    let mut skipped = 0usize;
    for rule in &rules {
        if let Some(pat) = &rule.pattern {
            regex_count += 1;
            if SKIP.contains(&rule.id.as_str()) {
                // Pattern is preserved on the rule; just not compilable by `regex`.
                skipped += 1;
                continue;
            }
            assert!(
                regex::Regex::new(pat).is_ok(),
                "rule {} has invalid regex pattern: {}",
                rule.id,
                pat
            );
        }
    }
    assert!(
        regex_count >= 40,
        "Charter expects most rules to carry regex patterns, got {} regex-bearing rules",
        regex_count
    );
    assert!(
        skipped <= 2,
        "more rules are unsupported by `regex` crate than expected ({})",
        skipped
    );
}

#[test]
fn catalog_covers_six_languages() {
    let rules = extended_scan_rules();
    let mut langs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for rule in &rules {
        for l in &rule.languages {
            langs.insert(l.to_string());
        }
    }
    // Expect at least: Python, JavaScript, TypeScript, Rust, Go, Java
    for required in ["Python", "JavaScript", "TypeScript", "Rust", "Go", "Java"] {
        assert!(
            langs.contains(required),
            "catalog missing required language: {} (got: {:?})",
            required,
            langs
        );
    }
}

#[test]
fn setTimeout_string_rule_regex_matches_expected_input() {
    // This is the highest-risk fix site: original line was
    //   pattern: Some(r"setTimeout\s*\(\s*['\"]".to_string()),
    // which is a raw-string syntax error. After the rescue, the
    // pattern must still actually match `setTimeout("...")` and
    // `setTimeout('...')`.
    let rules = extended_scan_rules();
    let js004 = rules
        .iter()
        .find(|r| r.id == "JS004")
        .expect("JS004 (setTimeout with string) missing");
    let pat = js004.pattern.as_ref().expect("JS004 pattern must be Some");
    let re = regex::Regex::new(pat).expect("JS004 regex must compile");
    assert!(re.is_match(r#"setTimeout("alert(1)", 100)"#));
    assert!(re.is_match(r#"setTimeout('alert(1)', 100)"#));
    assert!(!re.is_match("setTimeout(fn, 100)"));
}

#[test]
fn rust_todo_rule_has_balanced_pattern() {
    // Original line 309 had a stray `)`:
    //   pattern: Some(r"todo!\s*\(").to_string()),
    // After rescue, RUST004 must still match `todo!(`.
    let rules = extended_scan_rules();
    let rust004 = rules
        .iter()
        .find(|r| r.id == "RUST004")
        .expect("RUST004 (todo!()) missing");
    let pat = rust004
        .pattern
        .as_ref()
        .expect("RUST004 pattern must be Some");
    let re = regex::Regex::new(pat).expect("RUST004 regex must compile");
    assert!(re.is_match("todo!()"));
    assert!(re.is_match("todo! ( )"));
}
