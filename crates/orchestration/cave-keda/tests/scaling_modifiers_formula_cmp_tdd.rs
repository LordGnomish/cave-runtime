// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD coverage for the ScalingModifiers formula engine — Cycle 2:
//! comparison operators, logical `&& || !` (and the `and`/`or`/`not`
//! word forms expr-lang accepts), ternary `?:`, and the top-level
//! float coercion KEDA applies via `castToFloatIfNecessary` +
//! `expr.AsFloat64()` (a bare comparison or ternary still yields a float).

use cave_keda::eval_formula;
use std::collections::BTreeMap;

fn vars(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn ev(formula: &str, pairs: &[(&str, f64)]) -> f64 {
    eval_formula(formula, &vars(pairs)).expect("formula should evaluate")
}

#[test]
fn ternary_picks_branch_on_comparison() {
    // Canonical KEDA scaling-modifier example.
    assert_eq!(ev("a > 2 ? a + b : 1", &[("a", 3.0), ("b", 4.0)]), 7.0);
    assert_eq!(ev("a > 2 ? a + b : 1", &[("a", 1.0), ("b", 4.0)]), 1.0);
}

#[test]
fn bare_comparison_coerces_to_float() {
    // expr.AsFloat64 → true == 1.0, false == 0.0.
    assert_eq!(ev("a > b", &[("a", 5.0), ("b", 3.0)]), 1.0);
    assert_eq!(ev("a > b", &[("a", 1.0), ("b", 3.0)]), 0.0);
}

#[test]
fn explicit_float_of_comparison() {
    assert_eq!(ev("float(a == b)", &[("a", 2.0), ("b", 2.0)]), 1.0);
    assert_eq!(ev("float(a != b)", &[("a", 2.0), ("b", 2.0)]), 0.0);
}

#[test]
fn all_comparison_operators() {
    assert_eq!(ev("a <= b", &[("a", 2.0), ("b", 2.0)]), 1.0);
    assert_eq!(ev("a >= b", &[("a", 2.0), ("b", 3.0)]), 0.0);
    assert_eq!(ev("a < b", &[("a", 2.0), ("b", 3.0)]), 1.0);
}

#[test]
fn logical_and_or_with_symbols_and_words() {
    assert_eq!(ev("a > 0 && b > 0 ? 1 : 0", &[("a", 1.0), ("b", 2.0)]), 1.0);
    assert_eq!(ev("a > 0 and b > 5 ? 1 : 0", &[("a", 1.0), ("b", 2.0)]), 0.0);
    assert_eq!(ev("a > 9 || b > 0 ? 1 : 0", &[("a", 1.0), ("b", 2.0)]), 1.0);
    assert_eq!(ev("a > 9 or b > 9 ? 1 : 0", &[("a", 1.0), ("b", 2.0)]), 0.0);
}

#[test]
fn logical_not() {
    assert_eq!(ev("!(a > b) ? 1 : 0", &[("a", 1.0), ("b", 5.0)]), 1.0);
    assert_eq!(ev("not (a > b) ? 1 : 0", &[("a", 9.0), ("b", 5.0)]), 0.0);
}

#[test]
fn comparison_binds_tighter_than_ternary() {
    // a + b > 5 must group as (a + b) > 5, not a + (b > 5).
    assert_eq!(ev("a + b > 5 ? 10 : 0", &[("a", 3.0), ("b", 4.0)]), 10.0);
    assert_eq!(ev("a + b > 5 ? 10 : 0", &[("a", 1.0), ("b", 1.0)]), 0.0);
}
