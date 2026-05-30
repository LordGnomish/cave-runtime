// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for the common-subexpression-elimination (CSE)
//! analysis ported from `datafusion-common/src/cse.rs`.
//!
//! Upstream CSE walks a forest of expressions, assigns each subtree a
//! structural identifier, counts how many times each identifier occurs,
//! and reports the subexpressions that occur more than once (the
//! candidates that an optimizer would hoist into a single shared
//! evaluation). This test pins that pure, in-crate analysis before the
//! `src/cse.rs` module exists.

use cave_datafusion::cse::CommonSubexprAnalysis;
use cave_datafusion::logical_expr::{BinaryOp, LogicalExpr};

// `a + b` appears twice across the two projection expressions, so it must
// be reported as a common subexpression. The leaf columns `a` and `b`
// appear twice each too, but DataFusion's CSE never hoists trivial leaf
// nodes (Column / Literal) — only compound expressions count.
fn ab() -> LogicalExpr {
    LogicalExpr::binary(
        LogicalExpr::col("a"),
        BinaryOp::Plus,
        LogicalExpr::col("b"),
    )
}

#[test]
fn detects_repeated_compound_subexpr() {
    // SELECT (a + b) * c, (a + b) - 1
    let e1 = LogicalExpr::binary(ab(), BinaryOp::Multiply, LogicalExpr::col("c"));
    let e2 = LogicalExpr::binary(ab(), BinaryOp::Minus, LogicalExpr::lit(1));

    let analysis = CommonSubexprAnalysis::analyze(&[e1, e2]);

    // `a + b` is the one common subexpression.
    let commons = analysis.common_exprs();
    assert_eq!(commons.len(), 1, "exactly one common subexpr expected");
    assert_eq!(commons[0], ab(), "the common subexpr must be `a + b`");

    // It occurred exactly twice.
    assert_eq!(analysis.occurrences(&ab()), 2);

    // A leaf column never qualifies as a CSE candidate even though it
    // appears multiple times.
    assert!(analysis.common_exprs().iter().all(|e| !matches!(
        e,
        LogicalExpr::Column { .. } | LogicalExpr::Literal { .. }
    )));
}

#[test]
fn no_common_when_all_distinct() {
    // SELECT a + b, c - d  — nothing repeats.
    let e1 = LogicalExpr::binary(
        LogicalExpr::col("a"),
        BinaryOp::Plus,
        LogicalExpr::col("b"),
    );
    let e2 = LogicalExpr::binary(
        LogicalExpr::col("c"),
        BinaryOp::Minus,
        LogicalExpr::col("d"),
    );
    let analysis = CommonSubexprAnalysis::analyze(&[e1, e2]);
    assert!(analysis.common_exprs().is_empty());
}

#[test]
fn nested_repeat_counts_each_occurrence() {
    // (a + b) + (a + b)  — the inner `a + b` occurs twice inside one expr,
    // and the whole `(a+b)+(a+b)` occurs once.
    let inner = ab();
    let whole = LogicalExpr::binary(inner.clone(), BinaryOp::Plus, inner.clone());
    let analysis = CommonSubexprAnalysis::analyze(std::slice::from_ref(&whole));

    assert_eq!(analysis.occurrences(&inner), 2);
    assert_eq!(analysis.occurrences(&whole), 1);
    let commons = analysis.common_exprs();
    assert_eq!(commons, vec![inner]);
}
