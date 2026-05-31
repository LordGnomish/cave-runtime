// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: Harbor tag-retention rule engine.
//!
//! Ports goharbor/harbor `src/pkg/retention/policy/{rule,alg/or}` — the OR
//! processor + the latestPushedK / latestPulledN / nDaysSinceLastPush /
//! nDaysSinceLastPull / always performers + doublestar/regexp tag & repository
//! selectors. The HTTP handler previously stubbed `execute_retention` to a bare
//! 201; this engine computes the actual retain/delete partition.

use cave_artifacts::harbor::retention::{
    Candidate, Decoration, Policy, Rule, Selector, SelectorKind, Template,
};
use chrono::{Duration, TimeZone, Utc};

fn at(days_ago: i64, now: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    now - Duration::days(days_ago)
}

fn cand(repo: &str, tag: &str, pushed_days_ago: i64, now: chrono::DateTime<Utc>) -> Candidate {
    Candidate {
        repository: repo.to_string(),
        tag: tag.to_string(),
        push_time: at(pushed_days_ago, now),
        pull_time: None,
        labels: vec![],
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap()
}

// ── Doublestar selector ───────────────────────────────────────────────────────

#[test]
fn doublestar_selector_matches_prefix_glob() {
    let s = Selector::doublestar(Decoration::Matches, "release-*");
    assert!(s.matches_tag("release-1.2.3"));
    assert!(s.matches_tag("release-"));
    assert!(!s.matches_tag("nightly-1"));
}

#[test]
fn doublestar_excludes_inverts_the_match() {
    let s = Selector::doublestar(Decoration::Excludes, "nightly-*");
    // excludes => a tag is "selected" when it does NOT match the pattern
    assert!(s.matches_tag("release-1"));
    assert!(!s.matches_tag("nightly-7"));
}

#[test]
fn doublestar_double_star_matches_everything() {
    let s = Selector::doublestar(Decoration::Matches, "**");
    assert!(s.matches_tag("anything"));
    assert!(s.matches_tag("a/b/c"));
}

#[test]
fn regexp_selector_full_anchored_match() {
    let s = Selector::regexp(Decoration::Matches, r"v\d+\.\d+");
    assert!(s.matches_tag("v1.2"));
    assert!(!s.matches_tag("v1.2-rc")); // anchored: trailing chars reject
    assert!(!s.matches_tag("xv1.2"));
}

// ── latestPushedK performer (per-repository) ──────────────────────────────────

#[test]
fn latest_pushed_k_keeps_the_k_newest_per_repository() {
    let n = now();
    let cands = vec![
        cand("app", "v1", 30, n),
        cand("app", "v2", 20, n),
        cand("app", "v3", 10, n), // newest
        cand("app", "v4", 5, n),  // newest-1
    ];
    let policy = Policy {
        rules: vec![Rule {
            disabled: false,
            template: Template::LatestPushedK(2),
            scope_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
            tag_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
        }],
    };
    let out = policy.evaluate(&cands, n);
    let kept: Vec<_> = out.retained.iter().map(|c| c.tag.as_str()).collect();
    let del: Vec<_> = out.deleted.iter().map(|c| c.tag.as_str()).collect();
    assert!(kept.contains(&"v4") && kept.contains(&"v3"), "kept={:?}", kept);
    assert!(del.contains(&"v1") && del.contains(&"v2"), "del={:?}", del);
    assert_eq!(out.deleted.len(), 2);
}

#[test]
fn latest_pushed_k_is_scoped_per_repository_not_global() {
    let n = now();
    let cands = vec![
        cand("app", "a1", 9, n),
        cand("app", "a2", 1, n),
        cand("lib", "b1", 8, n),
        cand("lib", "b2", 2, n),
    ];
    // keep 1 newest PER repo => a2 + b2 kept, a1 + b1 deleted
    let policy = Policy {
        rules: vec![Rule {
            disabled: false,
            template: Template::LatestPushedK(1),
            scope_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
            tag_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
        }],
    };
    let out = policy.evaluate(&cands, n);
    let del: Vec<_> = out.deleted.iter().map(|c| c.tag.clone()).collect();
    assert_eq!(out.deleted.len(), 2, "del={:?}", del);
    assert!(del.contains(&"a1".to_string()) && del.contains(&"b1".to_string()));
}

// ── nDaysSinceLastPush performer ──────────────────────────────────────────────

#[test]
fn n_days_since_last_push_keeps_recent_deletes_old() {
    let n = now();
    let cands = vec![
        cand("app", "fresh", 3, n), // within 7d
        cand("app", "stale", 30, n),
    ];
    let policy = Policy {
        rules: vec![Rule {
            disabled: false,
            template: Template::NDaysSinceLastPush(7),
            scope_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
            tag_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
        }],
    };
    let out = policy.evaluate(&cands, n);
    assert_eq!(out.deleted.len(), 1);
    assert_eq!(out.deleted[0].tag, "stale");
}

// ── Scope/tag selectors carve the deletable universe ──────────────────────────

#[test]
fn tags_outside_every_rule_scope_are_left_untouched() {
    let n = now();
    let cands = vec![
        cand("app", "release-1", 40, n),
        cand("app", "release-2", 1, n),
        cand("app", "hotfix-9", 99, n), // matches no rule's tag selector
    ];
    // rule only scopes release-* tags, keep latest 1; hotfix-* untouched
    let policy = Policy {
        rules: vec![Rule {
            disabled: false,
            template: Template::LatestPushedK(1),
            scope_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
            tag_selectors: vec![Selector::doublestar(Decoration::Matches, "release-*")],
        }],
    };
    let out = policy.evaluate(&cands, n);
    let del: Vec<_> = out.deleted.iter().map(|c| c.tag.clone()).collect();
    assert_eq!(del, vec!["release-1".to_string()], "del={:?}", del);
    // hotfix-9 must survive even though it is ancient and unmatched
    assert!(out.retained.iter().any(|c| c.tag == "hotfix-9"));
}

// ── OR combination across rules ───────────────────────────────────────────────

#[test]
fn or_processor_retains_if_any_rule_retains() {
    let n = now();
    let cands = vec![
        cand("app", "v-old", 50, n),
        cand("app", "v-new", 1, n),
    ];
    // rule A: keep latest 1 (=> v-new). rule B: always-retain anything tagged v-old.
    let policy = Policy {
        rules: vec![
            Rule {
                disabled: false,
                template: Template::LatestPushedK(1),
                scope_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
                tag_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
            },
            Rule {
                disabled: false,
                template: Template::Always,
                scope_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
                tag_selectors: vec![Selector::doublestar(Decoration::Matches, "v-old")],
            },
        ],
    };
    let out = policy.evaluate(&cands, n);
    // v-old retained by rule B even though rule A would drop it => nothing deleted
    assert_eq!(out.deleted.len(), 0, "deleted={:?}", out.deleted);
    assert_eq!(out.retained.len(), 2);
}

#[test]
fn disabled_rule_is_ignored() {
    let n = now();
    let cands = vec![cand("app", "v1", 50, n), cand("app", "v2", 1, n)];
    let policy = Policy {
        rules: vec![Rule {
            disabled: true,
            template: Template::LatestPushedK(1),
            scope_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
            tag_selectors: vec![Selector::doublestar(Decoration::Matches, "**")],
        }],
    };
    let out = policy.evaluate(&cands, n);
    // no enabled rule => nothing is in scope => nothing deleted
    assert_eq!(out.deleted.len(), 0);
    assert_eq!(out.retained.len(), 2);
}

#[test]
fn selector_kind_label_matches_on_labels() {
    let _ = SelectorKind::Label; // kind enum exists
    let mut c = cand("app", "v1", 1, now());
    c.labels = vec!["keep".to_string()];
    let s = Selector::label(Decoration::Matches, "keep");
    assert!(s.matches_candidate(&c));
    let s2 = Selector::label(Decoration::Matches, "prod");
    assert!(!s2.matches_candidate(&c));
}
