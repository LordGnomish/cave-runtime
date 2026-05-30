// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of Velero restore resource-ordering logic:
//!   - pkg/types/priority.go  (Priorities::{parse, to_string})
//!   - pkg/restore/restore.go (getOrderedResources)
//! 2026-05-30 RED commit references not-yet-existing module cave_backup::restore_order.

use cave_backup::restore_order::{get_ordered_resources, Priorities};

// ── Mirror of Velero TestStringOfPriorities (pkg/types/priority_test.go) ──────

#[test]
fn string_high_only() {
    let p = Priorities {
        high: vec!["high".into()],
        low: vec![],
    };
    assert_eq!(p.to_priority_string(), "high");
}

#[test]
fn string_high_and_low() {
    let p = Priorities {
        high: vec!["high".into()],
        low: vec!["low".into()],
    };
    assert_eq!(p.to_priority_string(), "high,-,low");
}

// ── Mirror of Velero TestSetOfPriority ────────────────────────────────────────

#[test]
fn parse_empty_input() {
    let p = Priorities::parse("").unwrap();
    assert!(p.high.is_empty() && p.low.is_empty());
}

#[test]
fn parse_only_high() {
    let p = Priorities::parse("p0").unwrap();
    assert_eq!(p.high, vec!["p0".to_string()]);
    assert!(p.low.is_empty());
}

#[test]
fn parse_only_low() {
    let p = Priorities::parse("-,p9").unwrap();
    assert!(p.high.is_empty());
    assert_eq!(p.low, vec!["p9".to_string()]);
}

#[test]
fn parse_only_separator() {
    let p = Priorities::parse("-").unwrap();
    assert!(p.high.is_empty() && p.low.is_empty());
}

#[test]
fn parse_multiple_separators_errors() {
    assert!(Priorities::parse("-,-").is_err());
}

#[test]
fn parse_both_high_and_low() {
    let p = Priorities::parse("p0,p1,p2,-,p9").unwrap();
    assert_eq!(p.high, vec!["p0", "p1", "p2"]);
    assert_eq!(p.low, vec!["p9"]);
}

#[test]
fn parse_end_with_separator() {
    let p = Priorities::parse("p0,-").unwrap();
    assert_eq!(p.high, vec!["p0"]);
    assert!(p.low.is_empty());
}

// ── Mirror of Velero getOrderedResources semantics ───────────────────────────

#[test]
fn ordered_high_then_alpha_middle_then_low() {
    let priorities = Priorities {
        high: vec!["customresourcedefinitions".into(), "namespaces".into()],
        low: vec!["webhookconfigurations".into()],
    };
    let backup_resources = vec![
        "pods".to_string(),
        "services".to_string(),
        "configmaps".to_string(),
        // these appear in priorities and must be filtered out of the middle:
        "namespaces".to_string(),
        "webhookconfigurations".to_string(),
    ];
    let ordered = get_ordered_resources(&priorities, &backup_resources);
    assert_eq!(
        ordered,
        vec![
            "customresourcedefinitions",
            "namespaces",
            // alphabetized middle (priorities removed):
            "configmaps",
            "pods",
            "services",
            "webhookconfigurations",
        ]
    );
}

#[test]
fn ordered_no_priorities_is_just_alphabetical() {
    let priorities = Priorities {
        high: vec![],
        low: vec![],
    };
    let backup_resources = vec!["zebra".to_string(), "apple".to_string(), "mango".to_string()];
    let ordered = get_ordered_resources(&priorities, &backup_resources);
    assert_eq!(ordered, vec!["apple", "mango", "zebra"]);
}
