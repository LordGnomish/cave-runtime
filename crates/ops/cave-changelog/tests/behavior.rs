// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for `cave_changelog::engine::parse_commit`.
//!
//! Upstream parity reference: twisted/towncrier @ 25.8.0
//! (`test_builder.py::TestParseNewsfragmentBasename` classifies a fragment
//! filename into a category; cave's analogue classifies a conventional-commit
//! message into a `ChangeType`). towncrier itself is a CLI news-fragment
//! assembler (VCS/templating/config/file-writer) — those surfaces are scope-cut
//! and absent from cave. The only portable analogue is the
//! classification/description-extraction logic in `engine::parse_commit`.
//!
//! These tests exercise the already-implemented but previously untested branches:
//! the `Deprecated` (deprecated/deprecate), `Removed` (remove/revert), and
//! `Changed` (refactor/style) alias branches, plus the `splitn(2, ':')`
//! description-extraction edge cases (multi-colon, missing-colon fallback,
//! original-case preservation). Expected values are derived directly from
//! `crates/ops/cave-changelog/src/engine.rs` lines 9–34.

use cave_changelog::engine::parse_commit;
use cave_changelog::models::ChangeType;

// --- Deprecated branch (engine.rs:22 — "deprecated" || "deprecate") ---

#[test]
fn test_parse_commit_deprecated_full() {
    let (ct, desc) = parse_commit("deprecated: drop old api").expect("deprecated should classify");
    assert_eq!(ct, ChangeType::Deprecated);
    assert_eq!(desc, "drop old api");
}

#[test]
fn test_parse_commit_deprecate_alias_with_scope() {
    // "deprecate(core): ..." — starts_with("deprecate") is true even though
    // the full "deprecated" prefix does not match.
    let (ct, desc) =
        parse_commit("deprecate(core): mark builder removed").expect("deprecate alias classifies");
    assert_eq!(ct, ChangeType::Deprecated);
    assert_eq!(desc, "mark builder removed");
}

// --- Removed branch (engine.rs:24 — "remove" || "revert") ---

#[test]
fn test_parse_commit_remove_full() {
    let (ct, desc) = parse_commit("remove: legacy module").expect("remove should classify");
    assert_eq!(ct, ChangeType::Removed);
    assert_eq!(desc, "legacy module");
}

#[test]
fn test_parse_commit_revert_alias() {
    // "revert" does not start with "remove"; it hits the second arm of the OR.
    let (ct, desc) = parse_commit("revert: bad merge").expect("revert alias should classify");
    assert_eq!(ct, ChangeType::Removed);
    assert_eq!(desc, "bad merge");
}

// --- Changed branch aliases (engine.rs:26-29 — refactor || style; chore already tested) ---

#[test]
fn test_parse_commit_refactor_is_changed() {
    let (ct, desc) = parse_commit("refactor: tidy internals").expect("refactor classifies");
    assert_eq!(ct, ChangeType::Changed);
    assert_eq!(desc, "tidy internals");
}

#[test]
fn test_parse_commit_style_is_changed() {
    let (ct, desc) = parse_commit("style: fmt").expect("style classifies");
    assert_eq!(ct, ChangeType::Changed);
    assert_eq!(desc, "fmt");
}

// --- Description extraction edge cases (engine.rs:11-15, splitn(2, ':')) ---

#[test]
fn test_parse_commit_only_first_colon_splits() {
    // splitn(2, ':') keeps everything after the first colon, trimmed.
    let (ct, desc) = parse_commit("feat: a: b").expect("feat classifies");
    assert_eq!(ct, ChangeType::Added);
    assert_eq!(desc, "a: b");
}

#[test]
fn test_parse_commit_no_colon_empty_desc() {
    // No ':' → nth(1) is None → unwrap_or_default() yields an empty String.
    let (ct, desc) = parse_commit("feat").expect("bare feat still classifies");
    assert_eq!(ct, ChangeType::Added);
    assert_eq!(desc, "");
}

#[test]
fn test_parse_commit_desc_preserves_original_case() {
    // Only the type prefix is matched case-insensitively (via `lower`);
    // the description is sliced from the original message, preserving case.
    let (ct, desc) = parse_commit("Fix: Resolve Login Issue").expect("Fix classifies");
    assert_eq!(ct, ChangeType::Fixed);
    assert_eq!(desc, "Resolve Login Issue");
}
