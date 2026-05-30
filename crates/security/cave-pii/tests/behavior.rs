// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for cave-pii.
//!
//! These pin behaviors ported from Microsoft Presidio v2.2.0 onto cave's
//! deliberately-minimal regex/redaction surface:
//!   - the redact/mask operator contract (presidio `operators/test_mask.py`,
//!     `operators/test_redact.py`): keep a fixed prefix/suffix, mask the middle,
//!     and collapse short values entirely;
//!   - email recognition across line positions (presidio
//!     `predefined_recognizers/test_email_recognizer.py`);
//!   - findings aggregation by entity type (presidio analyzer count assertions);
//!   - the snake_case wire format of the entity-type enum (presidio
//!     `RecognizerResult.to_dict` analog), which the JSON/API layer depends on.
//!
//! Every assertion checks a concrete value derived from the actual impl in
//! `src/engine.rs` and `src/models.rs`. Tests that would require an async
//! runtime (the health route) or unimplemented logic (Luhn checksum, IP/phone
//! recognition) are intentionally omitted — see the TDD gap audit.

use cave_pii::engine::{count_by_type, find_emails, redact};
use cave_pii::models::{PiiFinding, PiiScanResult, PiiType};
use uuid::Uuid;

fn finding(pii_type: PiiType, confidence: f32) -> PiiFinding {
    PiiFinding {
        detector_id: Uuid::nil(),
        pii_type,
        line_number: 1,
        redacted: "**".to_string(),
        confidence,
    }
}

// ---- redact / mask operator contract -------------------------------------

#[test]
fn test_redact_empty_and_exact_boundary() {
    // Empty value collapses to empty (presidio: redact of empty => empty).
    assert_eq!(redact(""), "");
    // len 4 is the `<= 4` branch boundary: every char masked, no kept chars.
    assert_eq!(redact("abcd"), "****");
    // len 5 is the first "long" case: keep first ("a") + last ("e"),
    // mask the middle len-4 = 1 char.
    assert_eq!(redact("abcde"), "a*e");
}

#[test]
fn test_redact_preserves_prefix_suffix_chars() {
    // 10-char ASCII value: keep first 2 + last 2, mask exactly len-4 = 6.
    let out = redact("0123456789");
    assert_eq!(out, "01******89");
    assert_eq!(out.len(), 10);
    assert_eq!(&out[..2], "01");
    assert_eq!(&out[out.len() - 2..], "89");
    assert_eq!(out.chars().filter(|c| *c == '*').count(), 6);
}

#[test]
fn test_redact_short_values_fully_masked() {
    // 1..=4 length values are masked char-for-char, no original chars survive.
    assert_eq!(redact("a"), "*");
    assert_eq!(redact("ab"), "**");
    assert_eq!(redact("abc"), "***");
    assert_eq!(redact("abcd"), "****");
}

// ---- email recognition across line positions -----------------------------

#[test]
fn test_find_emails_at_line_end() {
    // Email is the last whitespace token on the line; returns line 1 + token.
    let found = find_emails("please contact jane@corp.io");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0], (1, "jane@corp.io".to_string()));
}

#[test]
fn test_find_emails_multiline_reports_line_numbers() {
    // Emails on lines 1 and 3; line 2 has neither '@' nor '.' => skipped.
    let content = "alice@a.com is here\nplain text line\nbob@b.org too";
    let found = find_emails(content);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0], (1, "alice@a.com".to_string()));
    assert_eq!(found[1], (3, "bob@b.org".to_string()));
}

#[test]
fn test_find_emails_first_matching_token_per_line() {
    // A line may contain other '@'/'.' tokens; only the first whitespace token
    // containing BOTH '@' and '.' is returned for that line.
    let content = "noreply@x.io and second@y.io on one line";
    let found = find_emails(content);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0], (1, "noreply@x.io".to_string()));
}

// ---- count_by_type aggregation -------------------------------------------

#[test]
fn test_count_by_type_empty_is_empty_map() {
    let counts = count_by_type(&[]);
    assert!(counts.is_empty());
}

#[test]
fn test_count_by_type_single_finding() {
    // Key is the Debug name of the variant: PiiType::Email => "Email".
    let counts = count_by_type(&[finding(PiiType::Email, 0.9)]);
    assert_eq!(counts.len(), 1);
    assert_eq!(counts.get("Email"), Some(&1));
    assert_eq!(counts.get("CreditCard"), None);
}

#[test]
fn test_count_by_type_uses_debug_variant_keys() {
    // Two distinct variants => two keys; key strings are the Rust variant
    // identifiers (Debug), not the snake_case serde wire form.
    let counts = count_by_type(&[
        finding(PiiType::SocialSecurityNumber, 0.8),
        finding(PiiType::SocialSecurityNumber, 0.8),
        finding(PiiType::IpAddress, 0.5),
    ]);
    assert_eq!(counts.get("SocialSecurityNumber"), Some(&2));
    assert_eq!(counts.get("IpAddress"), Some(&1));
    // snake_case form is NOT used as the map key.
    assert_eq!(counts.get("social_security_number"), None);
}

// ---- PiiType snake_case wire format ---------------------------------------

#[test]
fn test_pii_type_serde_snake_case() {
    // #[serde(rename_all = "snake_case")] contract the JSON/API layer relies on.
    let cases = [
        (PiiType::Email, "\"email\""),
        (PiiType::PhoneNumber, "\"phone_number\""),
        (PiiType::SocialSecurityNumber, "\"social_security_number\""),
        (PiiType::CreditCard, "\"credit_card\""),
        (PiiType::IpAddress, "\"ip_address\""),
        (PiiType::Name, "\"name\""),
        (PiiType::Address, "\"address\""),
    ];
    for (variant, wire) in cases {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, wire);
        // Round-trip back to the same variant.
        let back: PiiType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, variant);
    }
}

// ---- PiiScanResult serde round-trip ---------------------------------------

#[test]
fn test_pii_scan_result_serde_roundtrip() {
    // PiiScanResult is a plain data carrier; verify its serde derive preserves
    // every field (findings vec, total, high-confidence flag) across a round
    // trip, and that nested PiiFinding/PiiType serialize as snake_case.
    let result = PiiScanResult {
        findings: vec![finding(PiiType::CreditCard, 0.95)],
        total_findings: 1,
        has_high_confidence_pii: true,
    };
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["total_findings"], 1);
    assert_eq!(json["has_high_confidence_pii"], true);
    assert_eq!(json["findings"][0]["pii_type"], "credit_card");
    assert_eq!(json["findings"][0]["line_number"], 1);

    let back: PiiScanResult = serde_json::from_value(json).unwrap();
    assert_eq!(back.total_findings, 1);
    assert!(back.has_high_confidence_pii);
    assert_eq!(back.findings.len(), 1);
    assert_eq!(back.findings[0].pii_type, PiiType::CreditCard);
}
