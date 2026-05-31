// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD coverage for ScalingModifiersEvaluator — Cycle 4: the
//! composite-metric path must run the real expr-lang formula engine
//! (`eval_formula`), not the old prefix-only max/min/sum(name-list)
//! matcher. KEDA stores the formula result as the composite external
//! metric (`ret.Value.SetMilli`), then the HPA divides by the target.

use cave_keda::{ScalingModifiersEvaluator, Trigger};

fn evaluator(formula: &str, target: f64, triggers: &[(&str, f64)]) -> ScalingModifiersEvaluator {
    let mut ev = ScalingModifiersEvaluator::new();
    ev.formula = formula.to_string();
    ev.target = target;
    for (n, m) in triggers {
        ev.add_trigger(Trigger::new(n, *m, *m > 0.0));
    }
    ev
}

#[test]
fn arithmetic_formula_drives_composite_metric() {
    // (a + b) / 2 = 15 → ceil(15 / 5) = 3. The old matcher summed all
    // triggers (30) and returned 6.
    let ev = evaluator("(a + b) / 2", 5.0, &[("a", 10.0), ("b", 20.0)]);
    assert_eq!(ev.compute_metric(), 15.0);
    assert_eq!(ev.evaluate(), 3);
}

#[test]
fn ternary_formula_selects_branch() {
    // a > 5 ? 100 : 1 → 100 → ceil(100 / 10) = 10.
    let ev = evaluator("a > 5 ? 100 : 1", 10.0, &[("a", 10.0)]);
    assert_eq!(ev.compute_metric(), 100.0);
    assert_eq!(ev.evaluate(), 10);
}

#[test]
fn count_predicate_formula() {
    // count([a, b, c], {# > 1}) = 2 → ceil(2 / 1) = 2.
    let ev = evaluator(
        "count([a, b, c], {# > 1})",
        1.0,
        &[("a", 0.0), ("b", 2.0), ("c", 3.0)],
    );
    assert_eq!(ev.compute_metric(), 2.0);
    assert_eq!(ev.evaluate(), 2);
}

#[test]
fn empty_formula_falls_back_to_sum() {
    // No formula → composite metric is the sum of trigger metrics.
    let ev = evaluator("", 4.0, &[("a", 3.0), ("b", 5.0)]);
    assert_eq!(ev.compute_metric(), 8.0);
    assert_eq!(ev.evaluate(), 2);
}

#[test]
fn malformed_formula_falls_back_to_sum_safely() {
    // A formula referencing an undefined trigger must not panic — it
    // degrades to the sum of known metrics rather than crashing the loop.
    let ev = evaluator("ghost * 2", 1.0, &[("a", 4.0)]);
    assert_eq!(ev.compute_metric(), 4.0);
}
