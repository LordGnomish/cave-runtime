// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD parity port of TruffleHog `pkg/output/plain.go` (v3.63.7).
//!
//! The PlainPrinter emits a fixed-shape plain-text block per finding:
//!   "Found {verified|unverified} result 🐷🔑[❓]"
//!   "Detector Type: <type>"
//!   "Decoder Type: <decoder>"
//!   "Raw result: <raw>"
//!   <Title-cased metadata key>: <value>   (aggregate metadata, sorted)
//! RED first: `cave_secrets::output` does not exist yet.

use cave_secrets::models::{Confidence, SecretFinding, SecretType};
use cave_secrets::output::{plain_print, title_case, PlainResult};
use std::collections::BTreeMap;

fn base() -> PlainResult {
    PlainResult {
        detector_type: "AWS".to_string(),
        decoder_type: "PLAIN".to_string(),
        verified: true,
        raw: "AKIAIOSFODNN7EXAMPLE".to_string(),
        extra_data: BTreeMap::new(),
    }
}

#[test]
fn verified_header_and_core_labels() {
    let out = plain_print(&base());
    assert!(out.starts_with("Found verified result 🐷🔑\n"), "got: {out}");
    assert!(out.contains("Detector Type: AWS\n"));
    assert!(out.contains("Decoder Type: PLAIN\n"));
    assert!(out.contains("Raw result: AKIAIOSFODNN7EXAMPLE\n"));
}

#[test]
fn unverified_header_uses_question_emoji() {
    let mut r = base();
    r.verified = false;
    let out = plain_print(&r);
    assert!(
        out.starts_with("Found unverified result 🐷🔑❓\n"),
        "got: {out}"
    );
}

#[test]
fn metadata_keys_are_title_cased_and_sorted() {
    let mut r = base();
    r.extra_data
        .insert("repository".to_string(), "github.com/o/r".to_string());
    r.extra_data.insert("file".to_string(), "a.env".to_string());
    r.extra_data.insert("line".to_string(), "5".to_string());
    let out = plain_print(&r);
    // Title-cased labels present.
    assert!(out.contains("File: a.env\n"), "got: {out}");
    assert!(out.contains("Line: 5\n"), "got: {out}");
    assert!(out.contains("Repository: github.com/o/r\n"), "got: {out}");
    // Sorted: File < Line < Repository by key.
    let fi = out.find("File:").unwrap();
    let li = out.find("Line:").unwrap();
    let ri = out.find("Repository:").unwrap();
    assert!(fi < li && li < ri, "metadata not sorted: {out}");
}

#[test]
fn trailing_blank_line() {
    // Upstream prints fmt.Println("") at the end → a final newline-only line.
    let out = plain_print(&base());
    assert!(out.ends_with('\n'));
    assert!(out.ends_with("\n\n"), "expected trailing blank line: {out:?}");
}

#[test]
fn title_case_capitalizes_first_letter_lowercases_rest() {
    assert_eq!(title_case("repository"), "Repository");
    assert_eq!(title_case("FILE"), "File");
    assert_eq!(title_case("commit hash"), "Commit Hash");
    assert_eq!(title_case(""), "");
}

#[test]
fn from_finding_maps_fields_and_trims_raw() {
    let f = SecretFinding {
        id: "x".to_string(),
        rule_id: "aws-key".to_string(),
        rule_name: "AWS Access Key".to_string(),
        secret_type: SecretType::AwsCredential,
        file_path: "config.env".to_string(),
        line_number: Some(5),
        column: None,
        redacted_value: "AKIA****".to_string(),
        entropy: 4.0,
        confidence: Confidence::High,
        context: "  AWS_KEY=AKIA...  ".to_string(),
        commit: None,
    };
    let r = PlainResult::from_finding(&f, false);
    assert_eq!(r.detector_type, "aws_credential");
    assert_eq!(r.decoder_type, "PLAIN");
    assert!(!r.verified);
    // Raw is TrimSpace'd like upstream outputFormat.Raw.
    assert_eq!(r.raw, "AWS_KEY=AKIA...");
    assert_eq!(r.extra_data.get("file").map(String::as_str), Some("config.env"));
    assert_eq!(r.extra_data.get("line").map(String::as_str), Some("5"));
}
