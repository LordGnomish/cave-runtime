// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of Velero pkg/util/collections/includes_excludes.go
//! (IncludesExcludes.ShouldInclude / IncludeEverything + ValidateIncludesExcludes).
//! 2026-05-30 RED commit references not-yet-existing module cave_backup::includes_excludes.

use cave_backup::includes_excludes::{validate_includes_excludes, IncludesExcludes};

fn build(includes: &[&str], excludes: &[&str]) -> IncludesExcludes {
    let mut ie = IncludesExcludes::new();
    ie.includes(includes.iter().map(|s| s.to_string()).collect());
    ie.excludes(excludes.iter().map(|s| s.to_string()).collect());
    ie
}

// ── Mirror of Velero TestShouldInclude (includes_excludes_test.go) ────────────

#[test]
fn empty_string_should_include_every_item() {
    let ie = build(&[], &[]);
    assert!(ie.should_include("foo"));
}

#[test]
fn include_star_should_include_every_item() {
    let ie = build(&["*"], &[]);
    assert!(ie.should_include("foo"));
}

#[test]
fn item_in_includes_list_should_include_item() {
    let ie = build(&["foo", "bar", "baz"], &[]);
    assert!(ie.should_include("foo"));
}

#[test]
fn item_not_in_includes_list_should_not_include_item() {
    let ie = build(&["foo", "baz"], &[]);
    assert!(!ie.should_include("bar"));
}

#[test]
fn include_star_excluded_item_should_not_include_item() {
    let ie = build(&["*"], &["foo"]);
    assert!(!ie.should_include("foo"));
}

#[test]
fn include_star_exclude_foo_bar_should_be_included() {
    let ie = build(&["*"], &["foo"]);
    assert!(ie.should_include("bar"));
}

#[test]
fn item_both_included_and_excluded_should_not_be_included() {
    let ie = build(&["foo"], &["foo"]);
    assert!(!ie.should_include("foo"));
}

#[test]
fn wildcard_should_include_item() {
    let ie = build(&["*.bar"], &[]);
    assert!(ie.should_include("foo.bar"));
}

#[test]
fn wildcard_mismatch_should_not_include_item() {
    let ie = build(&["*.bar"], &[]);
    assert!(!ie.should_include("bar.foo"));
}

#[test]
fn wildcard_exclude_should_not_include_item() {
    let ie = build(&["*"], &["*.bar"]);
    assert!(!ie.should_include("foo.bar"));
}

#[test]
fn wildcard_exclude_mismatch_should_include_item() {
    let ie = build(&["*"], &["*.bar"]);
    assert!(ie.should_include("bar.foo"));
}

// ── IncludeEverything ─────────────────────────────────────────────────────────

#[test]
fn include_everything_empty() {
    assert!(build(&[], &[]).include_everything());
}

#[test]
fn include_everything_star_only() {
    assert!(build(&["*"], &[]).include_everything());
}

#[test]
fn include_everything_false_with_exclude() {
    assert!(!build(&["*"], &["foo"]).include_everything());
}

#[test]
fn include_everything_false_with_named_include() {
    assert!(!build(&["foo"], &[]).include_everything());
}

// ── Mirror of Velero TestValidateIncludesExcludes ─────────────────────────────

#[test]
fn validate_empty_includes_is_allowed() {
    assert!(validate_includes_excludes(&[], &[]).is_empty());
}

#[test]
fn validate_include_everything_allowed() {
    assert!(validate_includes_excludes(&["*".into()], &[]).is_empty());
}

#[test]
fn validate_star_with_other_includes_errors() {
    let errs = validate_includes_excludes(&["*".into(), "foo".into()], &[]);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("includes list must either contain '*' only"));
}

#[test]
fn validate_exclude_star_errors() {
    let errs = validate_includes_excludes(&["foo".into()], &["*".into()]);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("excludes list cannot contain '*'"));
}

#[test]
fn validate_exclude_overlaps_include_errors() {
    let errs = validate_includes_excludes(&["foo".into(), "bar".into()], &["bar".into()]);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("excludes list cannot contain an item in the includes list: bar"));
}
