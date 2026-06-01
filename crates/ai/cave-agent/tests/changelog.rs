// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-improvement step 3 — upstream changelog watch: semver parse/compare,
//! changelog parsing + entry classification, and "actionable since" filtering.

use cave_agent::changelog::{
    actionable_since, has_breaking_since, parse_changelog, ChangeKind, Version,
};
use cave_agent::AgentError;

const SAMPLE: &str = "\
## v2.1.0
- feat: composable parallel patterns
- fix: clamp temperature at zero

## v2.0.0
- BREAKING: rename tool registry API
- feat: plan-and-execute loop

## v1.9.0
- fix: percentile off-by-one
";

#[test]
fn version_parses_with_and_without_v_prefix() {
    assert_eq!(Version::parse("v1.2.3").unwrap(), Version::parse("1.2.3").unwrap());
    let v = Version::parse("v2.10.0").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (2, 10, 0));
}

#[test]
fn version_ordering_is_semver() {
    assert!(Version::parse("v2.0.0").unwrap() > Version::parse("v1.9.9").unwrap());
    assert!(Version::parse("v2.1.0").unwrap() > Version::parse("v2.0.5").unwrap());
    assert!(Version::parse("v2.0.10").unwrap() > Version::parse("v2.0.2").unwrap());
}

#[test]
fn invalid_version_is_parse_error() {
    assert!(matches!(Version::parse("two.point.oh"), Err(AgentError::Parse(_))));
    assert!(matches!(Version::parse("v1.2"), Err(AgentError::Parse(_))));
}

#[test]
fn parse_changelog_extracts_versioned_entries() {
    let entries = parse_changelog(SAMPLE);
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[0].version, Version::parse("v2.1.0").unwrap());
    assert_eq!(entries[0].kind, ChangeKind::Feature);
    assert_eq!(entries[2].kind, ChangeKind::Breaking);
    assert!(entries[0].summary.contains("parallel patterns"));
}

#[test]
fn actionable_since_returns_only_newer() {
    let entries = parse_changelog(SAMPLE);
    let cur = Version::parse("v2.0.0").unwrap();
    let act = actionable_since(&entries, &cur);
    // only the v2.1.0 entries are newer than v2.0.0
    assert_eq!(act.len(), 2);
    assert!(act.iter().all(|e| e.version > cur));
}

#[test]
fn has_breaking_since_detects_breaking_upgrade() {
    let entries = parse_changelog(SAMPLE);
    assert!(has_breaking_since(&entries, &Version::parse("v1.9.0").unwrap()));
    // nothing breaking strictly after v2.0.0
    assert!(!has_breaking_since(&entries, &Version::parse("v2.0.0").unwrap()));
}

#[test]
fn classification_covers_other() {
    let entries = parse_changelog("## v1.0.0\n- docs: tidy readme\n");
    assert_eq!(entries[0].kind, ChangeKind::Other);
}
