// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD coverage for the ScalingModifiers formula engine — Cycle 3:
//! array literals `[a, b, c]`, the reduction builtins expr-lang exposes
//! (`sum`, `avg`/`mean`, `min`, `max`, `len`) and the scalar math
//! builtins (`abs`, `ceil`, `floor`, `round`), plus `count(arr, {# > k})`
//! with a predicate closure — the form KEDA's scaling-modifier docs use.

use cave_keda::eval_formula;
use std::collections::BTreeMap;

fn vars(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn ev(formula: &str, pairs: &[(&str, f64)]) -> f64 {
    eval_formula(formula, &vars(pairs)).expect("formula should evaluate")
}

#[test]
fn sum_avg_over_array() {
    assert_eq!(ev("sum([a, b, c])", &[("a", 1.0), ("b", 2.0), ("c", 3.0)]), 6.0);
    assert_eq!(ev("avg([a, b])", &[("a", 4.0), ("b", 6.0)]), 5.0);
    assert_eq!(ev("mean([a, b])", &[("a", 4.0), ("b", 6.0)]), 5.0);
}

#[test]
fn min_max_len_over_array() {
    assert_eq!(ev("min([a, b, c])", &[("a", 3.0), ("b", 1.0), ("c", 2.0)]), 1.0);
    assert_eq!(ev("max([a, b, c])", &[("a", 3.0), ("b", 1.0), ("c", 2.0)]), 3.0);
    assert_eq!(ev("len([a, b, c])", &[("a", 0.0), ("b", 0.0), ("c", 0.0)]), 3.0);
}

#[test]
fn scalar_math_builtins() {
    assert_eq!(ev("abs(-a)", &[("a", 5.0)]), 5.0);
    assert_eq!(ev("ceil(a / b)", &[("a", 7.0), ("b", 2.0)]), 4.0);
    assert_eq!(ev("floor(a / b)", &[("a", 7.0), ("b", 2.0)]), 3.0);
    assert_eq!(ev("round(a / b)", &[("a", 7.0), ("b", 2.0)]), 4.0);
}

#[test]
fn variadic_min_max_without_array() {
    assert_eq!(ev("max(a, b)", &[("a", 3.0), ("b", 7.0)]), 7.0);
    assert_eq!(ev("min(a, b, c)", &[("a", 3.0), ("b", 7.0), ("c", 1.0)]), 1.0);
}

#[test]
fn count_with_predicate_closure() {
    // count how many of the listed metrics exceed 1.
    assert_eq!(
        ev("count([a, b, c], {# > 1})", &[("a", 0.0), ("b", 2.0), ("c", 3.0)]),
        2.0
    );
}

#[test]
fn count_predicate_in_keda_doc_ternary() {
    // Canonical KEDA docs example shape:
    //   count([t1, t2], {# > 1}) > 1 ? 5 : 0
    assert_eq!(
        ev("count([a, b], {# > 1}) > 1 ? 5 : 0", &[("a", 2.0), ("b", 3.0)]),
        5.0
    );
    assert_eq!(
        ev("count([a, b], {# > 1}) > 1 ? 5 : 0", &[("a", 0.0), ("b", 3.0)]),
        0.0
    );
}

#[test]
fn count_without_predicate_is_length() {
    assert_eq!(ev("count([a, b, c])", &[("a", 9.0), ("b", 9.0), ("c", 9.0)]), 3.0);
}

#[test]
fn hash_outside_predicate_errors() {
    let r = eval_formula("# + 1", &vars(&[("a", 1.0)]));
    assert!(r.is_err(), "'#' is only valid inside a count() predicate");
}
