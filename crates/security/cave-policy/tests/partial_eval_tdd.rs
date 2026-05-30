// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: OPA partial evaluation — residual query generation.
//!
//! Upstream: open-policy-agent/opa v1.16.2 — topdown/partial.go.
//! `opa eval --partial` evaluates a query treating `unknowns` (default
//! `input`) as undefined, folds away ground conjuncts (dropping ones proven
//! true, failing the whole query when one is proven false) and returns the
//! surviving residual conjuncts. Previously `partial_eval` was a stub that
//! always returned an empty result.

use cave_policy::rego::PolicyEngine;

#[test]
fn test_partial_keeps_unknown_conjunct() {
    let engine = PolicyEngine::new();
    let res = engine
        .partial_eval("input.x > 5", None, &[])
        .expect("partial eval");
    // One residual query holding the single unknown-dependent conjunct.
    assert_eq!(res.queries.len(), 1, "exactly one residual query");
    assert_eq!(res.queries[0].len(), 1, "one surviving conjunct");
    let rendered = serde_json::to_string(&res.queries[0]).unwrap();
    assert!(
        rendered.contains("input.x"),
        "residual must reference the unknown: {rendered}"
    );
}

#[test]
fn test_partial_drops_true_ground_conjunct() {
    let engine = PolicyEngine::new();
    // `1 < 2` is provably true → dropped; `input.x > 5` survives.
    let res = engine
        .partial_eval("1 < 2; input.x > 5", None, &["input".to_string()])
        .expect("partial eval");
    assert_eq!(res.queries.len(), 1);
    assert_eq!(
        res.queries[0].len(),
        1,
        "the true ground conjunct must be folded away"
    );
}

#[test]
fn test_partial_unsat_on_false_ground_conjunct() {
    let engine = PolicyEngine::new();
    // `2 < 1` is provably false → whole conjunction unsatisfiable → no queries.
    let res = engine
        .partial_eval("2 < 1; input.x > 5", None, &["input".to_string()])
        .expect("partial eval");
    assert_eq!(
        res.queries.len(),
        0,
        "a false ground conjunct makes the query unsatisfiable"
    );
}

#[test]
fn test_partial_all_ground_true_is_trivial() {
    let engine = PolicyEngine::new();
    // Fully known, all true → one empty residual query (unconditionally true).
    let res = engine
        .partial_eval("1 < 2; 2 < 3", None, &["input".to_string()])
        .expect("partial eval");
    assert_eq!(res.queries.len(), 1);
    assert!(
        res.queries[0].is_empty(),
        "trivially-true query has an empty residual body"
    );
}

#[test]
fn test_partial_taints_var_bound_from_unknown() {
    let engine = PolicyEngine::new();
    // `y := input.x` binds y from an unknown → y is tainted, so `y > 10`
    // survives as residual too. Both conjuncts are unknown-dependent.
    let res = engine
        .partial_eval("y := input.x; y > 10", None, &["input".to_string()])
        .expect("partial eval");
    assert_eq!(res.queries.len(), 1);
    assert_eq!(
        res.queries[0].len(),
        2,
        "the assignment and the tainted comparison both survive"
    );
}
