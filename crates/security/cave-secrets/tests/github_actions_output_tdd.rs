// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD parity port of TruffleHog `pkg/output/github_actions.go` (v3.63.7).
//!
//! GitHubActionsPrinter emits a `::warning file=..,line=..,endLine=..::<msg>`
//! workflow command per finding, deduplicating on a sha256 of
//! "<decoder>:<detector>:<status>:<file>:<line>". RED first:
//! `cave_secrets::output::GitHubActionsPrinter` does not exist yet.

use cave_secrets::output::GitHubActionsPrinter;

#[test]
fn emits_warning_command_for_verified_plain() {
    let mut p = GitHubActionsPrinter::new();
    let out = p
        .print("AWS", "PLAIN", true, "config.env", 12)
        .expect("first print emits");
    assert_eq!(
        out,
        "::warning file=config.env,line=12,endLine=12::Found verified AWS result 🐷🔑\n"
    );
}

#[test]
fn unverified_status_in_message() {
    let mut p = GitHubActionsPrinter::new();
    let out = p.print("GitHub", "PLAIN", false, "a.txt", 3).unwrap();
    assert_eq!(
        out,
        "::warning file=a.txt,line=3,endLine=3::Found unverified GitHub result 🐷🔑\n"
    );
}

#[test]
fn non_plain_decoder_notes_encoding() {
    let mut p = GitHubActionsPrinter::new();
    let out = p.print("Slack", "BASE64", true, "x.go", 7).unwrap();
    assert_eq!(
        out,
        "::warning file=x.go,line=7,endLine=7::Found verified Slack result with BASE64 encoding 🐷🔑\n"
    );
}

#[test]
fn identical_finding_is_deduped() {
    let mut p = GitHubActionsPrinter::new();
    assert!(p.print("AWS", "PLAIN", true, "config.env", 12).is_some());
    // Exact same key → suppressed (upstream returns nil).
    assert!(p.print("AWS", "PLAIN", true, "config.env", 12).is_none());
}

#[test]
fn different_line_is_not_deduped() {
    let mut p = GitHubActionsPrinter::new();
    assert!(p.print("AWS", "PLAIN", true, "config.env", 12).is_some());
    assert!(p.print("AWS", "PLAIN", true, "config.env", 13).is_some());
}

#[test]
fn verified_flag_distinguishes_dedupe_key() {
    let mut p = GitHubActionsPrinter::new();
    assert!(p.print("AWS", "PLAIN", true, "config.env", 12).is_some());
    // Same file/line/detector but unverified → distinct status → not deduped.
    assert!(p.print("AWS", "PLAIN", false, "config.env", 12).is_some());
}

#[test]
fn dedupe_key_is_sha256_hex_of_components() {
    // The cache key is the lowercase sha256 hex of
    // "<decoder>:<detector>:<status>:<file>:<line>".
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"PLAIN:AWS:verified:config.env:12");
    let expected = hex::encode(h.finalize());
    assert_eq!(GitHubActionsPrinter::dedupe_key("AWS", "PLAIN", true, "config.env", 12), expected);
}
