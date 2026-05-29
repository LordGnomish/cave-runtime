// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

//! Strict-TDD coverage for real SHA-256 content verification in cave-artifacts.
//!
//! The legacy `verify_sha256` only checked that the expected digest *string*
//! was 64 lower-hex chars; it never hashed the supplied bytes. That is a real
//! integrity bug: any 64-hex string would "verify" against arbitrary content.
//! These tests pin the correct behavior — the function must hash `data` and
//! compare it to `expected_hex`.

use cave_artifacts::pulp::content::verify_sha256;
use sha2::{Digest, Sha256};

/// Compute the canonical lower-hex SHA-256 of `data` (test-side oracle).
fn true_sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

#[test]
fn verify_sha256_rejects_wrong_content() {
    let expected = true_sha256_hex(b"hello");

    // Correct content against its true digest must verify.
    assert!(
        verify_sha256(b"hello", &expected),
        "verify_sha256 must accept content matching its true digest"
    );

    // Different content against the digest of b\"hello\" must be rejected.
    assert!(
        !verify_sha256(b"goodbye", &expected),
        "verify_sha256 must reject content that does not hash to the expected digest"
    );
}

#[test]
fn verify_sha256_rejects_well_formed_but_wrong_digest() {
    // A perfectly-formatted 64-hex digest that is NOT the hash of the data
    // must be rejected (the legacy implementation accepted this).
    let bogus = "a".repeat(64);
    assert!(
        !verify_sha256(b"hello", &bogus),
        "a 64-hex string that is not the true digest must not verify"
    );
}

#[test]
fn verify_sha256_known_vector() {
    // RFC-style known answer: sha256("") = e3b0c442...
    let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    assert!(verify_sha256(b"", empty));
    assert!(!verify_sha256(b"x", empty));
}
