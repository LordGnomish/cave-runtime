// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD coverage for the ScalingModifiers formula engine — a
//! faithful port of KEDA's `pkg/scaling/modifiers/formula.go`, which
//! evaluates the user `formula` with `github.com/expr-lang/expr` over a
//! `map[string]float64` of trigger-name → metric value, wrapped in
//! `float(...)` and run with `expr.AsFloat64()`
//! (apis/keda/v1alpha1/scaledobject_webhook.go castToFloatIfNecessary +
//! validateScalingModifiersFormula).
//!
//! Cycle 1 — arithmetic core: number/float literals, variable lookup,
//! `+ - * / %`, unary minus, parentheses, operator precedence.

use cave_keda::scaling_modifiers::eval_formula;
use std::collections::BTreeMap;

fn vars(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn ev(formula: &str, pairs: &[(&str, f64)]) -> f64 {
    eval_formula(formula, &vars(pairs)).expect("formula should evaluate")
}

#[test]
fn add_two_variables() {
    assert_eq!(ev("a + b", &[("a", 2.0), ("b", 3.0)]), 5.0);
}

#[test]
fn average_with_parens_and_division() {
    // The canonical KEDA scaling-modifier example.
    assert_eq!(ev("(a + b) / 2", &[("a", 4.0), ("b", 6.0)]), 5.0);
}

#[test]
fn precedence_mul_before_add() {
    assert_eq!(ev("2 + 3 * 4", &[]), 14.0);
    assert_eq!(ev("a * b - c", &[("a", 2.0), ("b", 3.0), ("c", 1.0)]), 5.0);
}

#[test]
fn unary_minus() {
    assert_eq!(ev("-a", &[("a", 5.0)]), -5.0);
    assert_eq!(ev("a - -b", &[("a", 1.0), ("b", 2.0)]), 3.0);
}

#[test]
fn modulo_and_float_literal() {
    assert_eq!(ev("a % b", &[("a", 7.0), ("b", 3.0)]), 1.0);
    assert_eq!(ev("a / 2.0", &[("a", 9.0)]), 4.5);
}

#[test]
fn nested_parens() {
    assert_eq!(ev("((a + b) * c)", &[("a", 1.0), ("b", 2.0), ("c", 4.0)]), 12.0);
}

#[test]
fn unknown_variable_is_an_error() {
    let r = eval_formula("ghost + 1", &vars(&[("a", 1.0)]));
    assert!(r.is_err(), "referencing an undefined trigger must error");
}

#[test]
fn divide_by_zero_is_an_error() {
    let r = eval_formula("a / b", &vars(&[("a", 1.0), ("b", 0.0)]));
    assert!(r.is_err(), "division by zero must surface as an error");
}
