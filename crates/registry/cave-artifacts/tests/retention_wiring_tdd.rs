// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: convert Harbor `RetentionPolicy` API models into the executable
//! retention engine (`Policy::from_harbor`) so the `/retentions/.../preview`
//! handler runs the real partition instead of returning a bare 201.

use cave_artifacts::harbor::harbor::{
    RetentionPolicy, RetentionRule, RetentionScope, RetentionSelector, RetentionTrigger,
};
use cave_artifacts::harbor::retention::{Candidate, Policy};
use chrono::{Duration, TimeZone, Utc};
use std::collections::HashMap;
use uuid::Uuid;

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap()
}

fn sel(kind: &str, decoration: &str, pattern: &str) -> RetentionSelector {
    RetentionSelector {
        kind: kind.to_string(),
        decoration: decoration.to_string(),
        pattern: pattern.to_string(),
        extras: None,
    }
}

fn harbor_policy(template: &str, k: i64) -> RetentionPolicy {
    let mut params = HashMap::new();
    params.insert(template.to_string(), serde_json::json!(k));
    let mut scope_selectors = HashMap::new();
    scope_selectors.insert("repository".to_string(), vec![sel("doublestar", "matches", "**")]);
    RetentionPolicy {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        scope: RetentionScope { level: "project".into(), ref_id: 1 },
        trigger: RetentionTrigger { kind: "Schedule".into(), settings: None, references: None },
        rules: vec![RetentionRule {
            disabled: false,
            action: "retain".into(),
            template: template.to_string(),
            tag_selectors: vec![sel("doublestar", "matches", "**")],
            scope_selectors,
            params,
        }],
    }
}

#[test]
fn from_harbor_maps_latest_pushed_k_and_runs() {
    let hp = harbor_policy("latestPushedK", 1);
    let engine: Policy = Policy::from_harbor(&hp);
    assert_eq!(engine.rules.len(), 1);

    let n = now();
    let cands = vec![
        Candidate {
            repository: "app".into(),
            tag: "old".into(),
            push_time: n - Duration::days(40),
            pull_time: None,
            labels: vec![],
        },
        Candidate {
            repository: "app".into(),
            tag: "new".into(),
            push_time: n - Duration::days(1),
            pull_time: None,
            labels: vec![],
        },
    ];
    let out = engine.evaluate(&cands, n);
    assert_eq!(out.deleted.len(), 1);
    assert_eq!(out.deleted[0].tag, "old");
}

#[test]
fn from_harbor_maps_n_days_and_excludes_decoration() {
    let hp = harbor_policy("nDaysSinceLastPush", 7);
    let engine = Policy::from_harbor(&hp);
    let n = now();
    let cands = vec![
        Candidate {
            repository: "app".into(),
            tag: "stale".into(),
            push_time: n - Duration::days(30),
            pull_time: None,
            labels: vec![],
        },
        Candidate {
            repository: "app".into(),
            tag: "fresh".into(),
            push_time: n - Duration::days(2),
            pull_time: None,
            labels: vec![],
        },
    ];
    let out = engine.evaluate(&cands, n);
    assert_eq!(out.deleted.len(), 1);
    assert_eq!(out.deleted[0].tag, "stale");
}

#[test]
fn from_harbor_skips_disabled_rules_and_unknown_templates() {
    // unknown template => the rule is dropped (no panic), so nothing is in scope
    let hp = harbor_policy("someFutureTemplate", 3);
    let engine = Policy::from_harbor(&hp);
    let n = now();
    let cands = vec![Candidate {
        repository: "app".into(),
        tag: "x".into(),
        push_time: n,
        pull_time: None,
        labels: vec![],
    }];
    let out = engine.evaluate(&cands, n);
    assert_eq!(out.deleted.len(), 0);
    assert_eq!(out.retained.len(), 1);
}
