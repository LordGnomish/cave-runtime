// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: Chaos Mesh selector semantics (api/v1alpha1/selector.go +
//! controllers/utils/selector). Pod selection modes, label matching operators,
//! namespace scoping, and exact subset-size formulas.

use cave_chaos::selector::{
    subset_size, resolve_targets, LabelExpr, LabelSelector, PodInfo, SelectorMode,
};
use std::collections::HashMap;

fn pod(name: &str, ns: &str, labels: &[(&str, &str)]) -> PodInfo {
    PodInfo {
        name: name.to_string(),
        namespace: ns.to_string(),
        labels: labels
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        healthy: true,
    }
}

fn pool(n: usize) -> Vec<PodInfo> {
    (0..n)
        .map(|i| pod(&format!("p{i}"), "default", &[("app", "web")]))
        .collect()
}

// ── subset_size mode formulas ───────────────────────────────────────────────

#[test]
fn test_mode_one_is_constant_one() {
    assert_eq!(subset_size(&SelectorMode::One, 100), 1);
    assert_eq!(subset_size(&SelectorMode::One, 1), 1);
}

#[test]
fn test_mode_one_empty_pool_is_zero() {
    assert_eq!(subset_size(&SelectorMode::One, 0), 0);
}

#[test]
fn test_mode_all_is_n() {
    assert_eq!(subset_size(&SelectorMode::All, 10), 10);
    assert_eq!(subset_size(&SelectorMode::All, 0), 0);
}

#[test]
fn test_mode_fixed_min_of_value_and_n() {
    // Example 5: N=10, fixed value=5 -> 5
    assert_eq!(subset_size(&SelectorMode::Fixed(5), 10), 5);
    // Example 6: N=3, fixed value=10 -> 3 (value exceeds pool)
    assert_eq!(subset_size(&SelectorMode::Fixed(10), 3), 3);
}

#[test]
fn test_mode_fixed_percent_uses_ceiling() {
    // Example 1: N=10, value=30 -> ceil(3.0)=3
    assert_eq!(subset_size(&SelectorMode::FixedPercent(30), 10), 3);
    // Example 2: N=10, value=33 -> ceil(3.3)=4
    assert_eq!(subset_size(&SelectorMode::FixedPercent(33), 10), 4);
    // Example 9: N=50, value=20 -> ceil(10.0)=10
    assert_eq!(subset_size(&SelectorMode::FixedPercent(20), 50), 10);
}

#[test]
fn test_mode_fixed_percent_capped_at_n() {
    assert_eq!(subset_size(&SelectorMode::FixedPercent(100), 7), 7);
    // 1% of 10 = ceil(0.1) = 1 (never zero for non-empty when percent>0)
    assert_eq!(subset_size(&SelectorMode::FixedPercent(1), 10), 1);
}

#[test]
fn test_mode_random_max_percent_uses_floor() {
    // Example 3: N=10, value=30 -> floor(3.0)=3
    assert_eq!(subset_size(&SelectorMode::RandomMaxPercent(30), 10), 3);
    // Example 4: N=10, value=33 -> floor(3.3)=3 (NOT 4)
    assert_eq!(subset_size(&SelectorMode::RandomMaxPercent(33), 10), 3);
}

// ── label matching ──────────────────────────────────────────────────────────

#[test]
fn test_match_labels_all_must_equal() {
    let sel = LabelSelector {
        match_labels: HashMap::from([
            ("app".to_string(), "web".to_string()),
            ("env".to_string(), "prod".to_string()),
        ]),
        match_expressions: vec![],
    };
    assert!(sel.matches(&pod("a", "default", &[("app", "web"), ("env", "prod")])));
    // missing one pair -> no match
    assert!(!sel.matches(&pod("b", "default", &[("app", "web")])));
    // wrong value -> no match
    assert!(!sel.matches(&pod("c", "default", &[("app", "web"), ("env", "dev")])));
}

#[test]
fn test_match_expression_in() {
    let sel = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelExpr::In(
            "env".to_string(),
            vec!["prod".to_string(), "staging".to_string()],
        )],
    };
    assert!(sel.matches(&pod("a", "default", &[("env", "prod")])));
    assert!(sel.matches(&pod("b", "default", &[("env", "staging")])));
    assert!(!sel.matches(&pod("c", "default", &[("env", "dev")])));
    // absent key fails In
    assert!(!sel.matches(&pod("d", "default", &[("app", "web")])));
}

#[test]
fn test_match_expression_notin_absent_key_passes() {
    // Example 10: NotIn(status,[error,failed]); pod with no status key matches.
    let sel = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelExpr::NotIn(
            "status".to_string(),
            vec!["error".to_string(), "failed".to_string()],
        )],
    };
    assert!(sel.matches(&pod("running", "default", &[("status", "running")])));
    assert!(!sel.matches(&pod("err", "default", &[("status", "error")])));
    assert!(sel.matches(&pod("nokey", "default", &[("app", "web")])));
}

#[test]
fn test_match_expression_exists_and_doesnotexist() {
    let exists = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelExpr::Exists("tier".to_string())],
    };
    assert!(exists.matches(&pod("a", "default", &[("tier", "frontend")])));
    assert!(!exists.matches(&pod("b", "default", &[("app", "web")])));

    let dne = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelExpr::DoesNotExist("tier".to_string())],
    };
    assert!(dne.matches(&pod("c", "default", &[("app", "web")])));
    assert!(!dne.matches(&pod("d", "default", &[("tier", "backend")])));
}

#[test]
fn test_match_labels_and_expressions_combined() {
    // Example 8: matchLabels{app:web,env:prod} AND Exists(tier)
    let sel = LabelSelector {
        match_labels: HashMap::from([
            ("app".to_string(), "web".to_string()),
            ("env".to_string(), "prod".to_string()),
        ]),
        match_expressions: vec![LabelExpr::Exists("tier".to_string())],
    };
    assert!(sel.matches(&pod(
        "a",
        "default",
        &[("app", "web"), ("env", "prod"), ("tier", "frontend")]
    )));
    // satisfies labels but missing tier -> fail
    assert!(!sel.matches(&pod("b", "default", &[("app", "web"), ("env", "prod")])));
}

// ── resolve_targets: namespace filter + label filter + mode reduction ────────

#[test]
fn test_resolve_namespace_filter_applies_first() {
    let pods = vec![
        pod("a", "default", &[("app", "web")]),
        pod("b", "prod", &[("app", "web")]),
        pod("c", "kube-system", &[("app", "web")]),
    ];
    let sel = LabelSelector {
        match_labels: HashMap::from([("app".to_string(), "web".to_string())]),
        match_expressions: vec![],
    };
    let out = resolve_targets(
        &pods,
        &["default".to_string(), "prod".to_string()],
        &sel,
        &SelectorMode::All,
    );
    let names: Vec<&str> = out.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b"]); // kube-system excluded by ns filter
}

#[test]
fn test_resolve_empty_namespaces_matches_all_namespaces() {
    let pods = vec![
        pod("a", "default", &[("app", "web")]),
        pod("b", "prod", &[("app", "web")]),
    ];
    let sel = LabelSelector {
        match_labels: HashMap::from([("app".to_string(), "web".to_string())]),
        match_expressions: vec![],
    };
    let out = resolve_targets(&pods, &[], &sel, &SelectorMode::All);
    assert_eq!(out.len(), 2);
}

#[test]
fn test_resolve_mode_reduces_after_filter_deterministic() {
    // 10 matching pods, fixed-percent 30 -> 3, deterministic input order.
    let pods = pool(10);
    let sel = LabelSelector {
        match_labels: HashMap::from([("app".to_string(), "web".to_string())]),
        match_expressions: vec![],
    };
    let out = resolve_targets(&pods, &[], &sel, &SelectorMode::FixedPercent(30));
    assert_eq!(out.len(), 3);
    let names: Vec<&str> = out.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["p0", "p1", "p2"]); // stable order, first-k
}

#[test]
fn test_resolve_no_matches_returns_empty() {
    let pods = pool(5);
    let sel = LabelSelector {
        match_labels: HashMap::from([("app".to_string(), "nomatch".to_string())]),
        match_expressions: vec![],
    };
    let out = resolve_targets(&pods, &[], &sel, &SelectorMode::All);
    assert!(out.is_empty());
}
