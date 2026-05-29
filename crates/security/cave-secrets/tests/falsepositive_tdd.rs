// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD: false-positive suppression ported from TruffleHog pkg/detectors/falsepositives.go.
//!
//! The existing `scan` fires on obvious placeholder/low-entropy values. A
//! TruffleHog-style filter drops results whose secret is a known false
//! positive (wordlist match, low-entropy, all-same-character) while keeping
//! realistic high-entropy secrets.

use cave_secrets::detector::{builtin_detectors, falsepositives, scan, scan_filtered, Finding};

/// A placeholder API key and a 24-char low-entropy value must be suppressed,
/// while a realistic high-entropy secret still produces a finding.
#[test]
fn known_false_positive_is_suppressed() {
    let detectors = builtin_detectors();

    // 1. Obvious placeholder — TruffleHog wordlist contains "example".
    //    Long enough (20+ chars) to trip the generic-api-key detector.
    let placeholder = r#"API_KEY="EXAMPLE_EXAMPLE_EXAMPLE_VALUE""#;
    // 2. Low-entropy 24x 'x' value — all-same-character, no real secret.
    let low_entropy = "api_key=xxxxxxxxxxxxxxxxxxxxxxxx";
    // 3. Realistic high-entropy secret that should survive the filter.
    let real = r#"api_key="A7xQ9pL2zR8vK3mN6wT1yB5dF4hG0jUq""#;

    // Unfiltered scan currently fires on the placeholder and the low-entropy value.
    let raw_ph = scan(placeholder, "p.env", &detectors);
    assert!(
        !raw_ph.is_empty(),
        "precondition: raw scan should fire on the placeholder"
    );

    // Filtered scan must suppress both false positives.
    let f_ph = scan_filtered(placeholder, "p.env", &detectors);
    assert!(
        f_ph.is_empty(),
        "placeholder 'example' must be suppressed, got {:?}",
        f_ph.iter().map(|f| &f.detector).collect::<Vec<_>>()
    );

    let f_low = scan_filtered(low_entropy, "p.env", &detectors);
    assert!(
        f_low.is_empty(),
        "low-entropy all-'x' value must be suppressed, got {:?}",
        f_low.iter().map(|f| &f.detector).collect::<Vec<_>>()
    );

    // The realistic secret must still be reported.
    let f_real = scan_filtered(real, "p.env", &detectors);
    assert!(
        !f_real.is_empty(),
        "a realistic high-entropy secret must NOT be suppressed"
    );
}

/// The standalone predicate ported from upstream `IsKnownFalsePositive`.
#[test]
fn falsepositive_predicate_matches_upstream() {
    // Wordlist hit (case-insensitive).
    assert!(falsepositives::is_known_false_positive("example"));
    assert!(falsepositives::is_known_false_positive("EXAMPLE"));
    assert!(falsepositives::is_known_false_positive("password"));
    assert!(falsepositives::is_known_false_positive("123456"));

    // All-same-character / low entropy.
    assert!(falsepositives::is_known_false_positive(
        "xxxxxxxxxxxxxxxxxxxxxxxx"
    ));

    // Realistic high-entropy secret is NOT a false positive.
    assert!(!falsepositives::is_known_false_positive(
        "A7xQ9pL2zR8vK3mN6wT1yB5dF4hG0jUq"
    ));
}

/// `scan_filtered` must be a strict subset of `scan` (never invent findings),
/// and must preserve the `Finding` shape.
#[test]
fn filtered_is_subset_of_raw() {
    let detectors = builtin_detectors();
    let content = r#"api_key="A7xQ9pL2zR8vK3mN6wT1yB5dF4hG0jUq""#;
    let raw: Vec<Finding> = scan(content, "x.env", &detectors);
    let filtered: Vec<Finding> = scan_filtered(content, "x.env", &detectors);
    assert!(filtered.len() <= raw.len());
}
