// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Argo Workflows `when` conditional evaluator —
//! `argoproj/argo-workflows v4.0.5`
//! (`workflow/controller/operator.go` `shouldExecute` + the expr-lang /
//! govaluate expression layer).
//!
//! A DAG task or Steps step may carry a `when:` clause such as
//! `"{{tasks.flip.outputs.result}} == heads"`. The controller substitutes the
//! `{{...}}` placeholders against the live node context, then evaluates the
//! remaining boolean expression. A task whose `when` resolves to `false` is
//! marked `Skipped` (a fulfilled phase that satisfies its dependents).
//!
//! This is a pure, dependency-light port: `{{...}}` substitution plus a small
//! recursive-descent evaluator over `||` / `&&` / comparison operators
//! (`== != =~ !~ < <= > >=`) with numeric-aware equality and regex matching.

use std::collections::HashMap;

/// Substitute `{{ key }}` placeholders in `expr` from `ctx`. Whitespace inside
/// the braces is trimmed. An unresolved placeholder is an error — mirroring
/// Argo, which fails the node rather than silently treating it as empty.
pub fn substitute(expr: &str, ctx: &HashMap<String, String>) -> Result<String, String> {
    // PLACEHOLDER (RED): returns the expression unchanged.
    let _ = ctx;
    Ok(expr.to_string())
}

/// Evaluate a `when` expression after `{{...}}` substitution. Returns the
/// boolean the controller uses to decide whether to execute the node.
pub fn evaluate_when(expr: &str, ctx: &HashMap<String, String>) -> Result<bool, String> {
    // PLACEHOLDER (RED): always true.
    let _ = (expr, ctx);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn substitute_resolves_placeholders() {
        let c = ctx(&[("tasks.flip.outputs.result", "heads")]);
        assert_eq!(
            substitute("{{tasks.flip.outputs.result}} == heads", &c).unwrap(),
            "heads == heads"
        );
    }

    #[test]
    fn substitute_trims_inner_whitespace() {
        let c = ctx(&[("x", "1")]);
        assert_eq!(substitute("{{ x }} > 0", &c).unwrap(), "1 > 0");
    }

    #[test]
    fn substitute_errors_on_unresolved() {
        let c = ctx(&[]);
        assert!(substitute("{{missing}} == 1", &c).is_err());
    }

    #[test]
    fn eval_string_equality() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("foo == foo", &c), Ok(true));
        assert_eq!(evaluate_when("foo == bar", &c), Ok(false));
    }

    #[test]
    fn eval_string_inequality() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("foo != bar", &c), Ok(true));
        assert_eq!(evaluate_when("foo != foo", &c), Ok(false));
    }

    #[test]
    fn eval_quoted_operands() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("\"a b\" == \"a b\"", &c), Ok(true));
        assert_eq!(evaluate_when("'x' != 'y'", &c), Ok(true));
    }

    #[test]
    fn eval_numeric_comparison() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("5 > 3", &c), Ok(true));
        assert_eq!(evaluate_when("2 > 3", &c), Ok(false));
        assert_eq!(evaluate_when("3 >= 3", &c), Ok(true));
        assert_eq!(evaluate_when("2 <= 1", &c), Ok(false));
        assert_eq!(evaluate_when("4 < 10", &c), Ok(true));
    }

    #[test]
    fn eval_numeric_aware_equality() {
        let c = ctx(&[]);
        // "5" == "5.0" — numeric-aware so equal even though the strings differ.
        assert_eq!(evaluate_when("5 == 5.0", &c), Ok(true));
    }

    #[test]
    fn eval_regex_match() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("heads =~ h.*", &c), Ok(true));
        assert_eq!(evaluate_when("tails =~ h.*", &c), Ok(false));
        assert_eq!(evaluate_when("tails !~ h.*", &c), Ok(true));
    }

    #[test]
    fn eval_boolean_and_or() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("a == a && b == b", &c), Ok(true));
        assert_eq!(evaluate_when("a == a && b == c", &c), Ok(false));
        assert_eq!(evaluate_when("a == x || b == b", &c), Ok(true));
        assert_eq!(evaluate_when("a == x || b == y", &c), Ok(false));
    }

    #[test]
    fn eval_precedence_or_of_ands() {
        let c = ctx(&[]);
        // (false && false) || true  ==>  true
        assert_eq!(evaluate_when("a == x && b == y || c == c", &c), Ok(true));
    }

    #[test]
    fn eval_bare_boolean_literal() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("true", &c), Ok(true));
        assert_eq!(evaluate_when("false", &c), Ok(false));
    }

    #[test]
    fn eval_end_to_end_with_substitution() {
        let c = ctx(&[("tasks.flip.outputs.result", "heads")]);
        assert_eq!(
            evaluate_when("{{tasks.flip.outputs.result}} == heads", &c),
            Ok(true)
        );
        let c2 = ctx(&[("tasks.flip.outputs.result", "tails")]);
        assert_eq!(
            evaluate_when("{{tasks.flip.outputs.result}} == heads", &c2),
            Ok(false)
        );
    }

    #[test]
    fn eval_numeric_lt_requires_numbers() {
        let c = ctx(&[]);
        assert!(evaluate_when("foo < bar", &c).is_err());
    }
}
