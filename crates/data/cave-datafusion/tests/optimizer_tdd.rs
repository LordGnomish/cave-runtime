// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for the logical optimizer ported from
//! `apache/datafusion` `datafusion-optimizer/`.
//!
//! DataFusion's `Optimizer` runs a set of rule passes to a fixpoint over
//! the `LogicalPlan` tree. This test pins the core rules the cave-datafusion
//! optimizer must carry — each is a faithful subset of an upstream rule:
//!   * constant folding / expression simplification
//!     (`datafusion-optimizer/src/simplify_expressions/`)
//!   * boolean short-circuit simplification (`x AND true` → `x`, etc.)
//!   * filter merging (`datafusion-optimizer/src/eliminate_filter` +
//!     the `push_down_filter` combine step)
//!   * predicate push-down below a pass-through projection
//!     (`datafusion-optimizer/src/push_down_filter.rs`)
//!   * limit push-down below a projection
//!     (`datafusion-optimizer/src/push_down_limit.rs`)
//!   * identity-projection elimination
//!     (`datafusion-optimizer/src/optimize_projections` /
//!     `eliminate_projection`)
//!
//! Pins the public surface before `src/optimizer.rs` exists (RED).

use cave_datafusion::logical_expr::{BinaryOp, LogicalExpr};
use cave_datafusion::logical_plan::LogicalPlan;
use cave_datafusion::optimizer::Optimizer;
use cave_datafusion::row::Value;
use cave_datafusion::schema::{DataType, Field, SchemaRef, TableSchema};
use std::sync::Arc;

fn schema_ab() -> SchemaRef {
    Arc::new(TableSchema::new(vec![
        Field::new("a", DataType::Int64, false),
        Field::new("b", DataType::Utf8, true),
    ]))
}

fn scan() -> LogicalPlan {
    LogicalPlan::TableScan {
        table_name: "t".into(),
        schema: schema_ab(),
        projection: None,
        filters: vec![],
    }
}

// `1 + 2` (both literals) must fold to the single literal `3`, matching
// the physical arithmetic semantics (integer + integer → Int64).
#[test]
fn constant_folding_folds_literal_arithmetic() {
    // Filter predicate: (1 + 2) = a
    let pred = LogicalExpr::binary(
        LogicalExpr::binary(LogicalExpr::lit(1), BinaryOp::Plus, LogicalExpr::lit(2)),
        BinaryOp::Eq,
        LogicalExpr::col("a"),
    );
    let plan = LogicalPlan::Filter {
        predicate: pred,
        input: Box::new(scan()),
    };
    let opt = Optimizer::new().optimize(plan);
    let LogicalPlan::Filter { predicate, .. } = opt else {
        panic!("expected Filter at root");
    };
    let LogicalExpr::BinaryOp { left, op, .. } = predicate else {
        panic!("expected BinaryOp predicate");
    };
    assert_eq!(op, BinaryOp::Eq);
    assert_eq!(
        *left,
        LogicalExpr::Literal {
            value: Value::Int64(3)
        },
        "1 + 2 must fold to the literal Int64(3)"
    );
}

// `x AND true` simplifies to `x`; `x OR false` simplifies to `x`.
#[test]
fn boolean_simplification_drops_identities() {
    let x = LogicalExpr::col("a").gt(LogicalExpr::lit(1));
    let pred = x.clone().and(LogicalExpr::lit(true));
    let plan = LogicalPlan::Filter {
        predicate: pred,
        input: Box::new(scan()),
    };
    let opt = Optimizer::new().optimize(plan);
    let LogicalPlan::Filter { predicate, .. } = opt else {
        panic!("expected Filter");
    };
    assert_eq!(predicate, x, "`x AND true` must simplify to `x`");
}

// Two stacked Filters merge into one conjunction.
#[test]
fn merge_consecutive_filters() {
    let p1 = LogicalExpr::col("a").gt(LogicalExpr::lit(1));
    let p2 = LogicalExpr::col("a").lt(LogicalExpr::lit(10));
    let plan = LogicalPlan::Filter {
        predicate: p1.clone(),
        input: Box::new(LogicalPlan::Filter {
            predicate: p2.clone(),
            input: Box::new(scan()),
        }),
    };
    let opt = Optimizer::new().optimize(plan);
    // Single Filter whose predicate is `p1 AND p2`, directly over the scan.
    let LogicalPlan::Filter { predicate, input } = opt else {
        panic!("expected a single merged Filter");
    };
    assert_eq!(predicate, p1.and(p2));
    assert!(
        matches!(*input, LogicalPlan::TableScan { .. }),
        "merged Filter must sit directly on the scan"
    );
}

// Filter above a pass-through Projection pushes below it. The projection
// reorders columns ([b, a]) so it is a genuine pass-through but NOT an
// identity projection — that keeps the push-down rule from racing with
// identity-projection elimination on the same input.
#[test]
fn push_down_filter_below_passthrough_projection() {
    // Filter(a > 1, Projection([b, a], scan))
    let plan = LogicalPlan::Filter {
        predicate: LogicalExpr::col("a").gt(LogicalExpr::lit(1)),
        input: Box::new(LogicalPlan::Projection {
            expressions: vec![LogicalExpr::col("b"), LogicalExpr::col("a")],
            input: Box::new(scan()),
        }),
    };
    let opt = Optimizer::new().optimize(plan);
    // Becomes Projection([b, a], Filter(a > 1, scan)).
    let LogicalPlan::Projection { input, .. } = opt else {
        panic!("expected Projection at root after push-down");
    };
    assert!(
        matches!(*input, LogicalPlan::Filter { .. }),
        "filter must be pushed below the projection"
    );
}

// Limit above a Projection pushes below it (projection is row-count
// preserving, so limiting earlier is safe and cheaper).
#[test]
fn push_down_limit_below_projection() {
    let plan = LogicalPlan::Limit {
        skip: 0,
        fetch: Some(5),
        input: Box::new(LogicalPlan::Projection {
            expressions: vec![LogicalExpr::col("a")],
            input: Box::new(scan()),
        }),
    };
    let opt = Optimizer::new().optimize(plan);
    let LogicalPlan::Projection { input, .. } = opt else {
        panic!("expected Projection at root after limit push-down");
    };
    assert!(
        matches!(*input, LogicalPlan::Limit { fetch: Some(5), .. }),
        "limit must be pushed below the projection"
    );
}

// A Projection that re-selects exactly the input columns in order is an
// identity and must be removed.
#[test]
fn eliminate_identity_projection() {
    let plan = LogicalPlan::Projection {
        expressions: vec![LogicalExpr::col("a"), LogicalExpr::col("b")],
        input: Box::new(scan()),
    };
    let opt = Optimizer::new().optimize(plan);
    assert!(
        matches!(opt, LogicalPlan::TableScan { .. }),
        "identity projection over the full schema must be eliminated"
    );
}

// The optimizer must be idempotent: optimizing an already-optimized plan
// is a no-op (fixpoint reached).
#[test]
fn optimizer_is_idempotent() {
    let plan = LogicalPlan::Filter {
        predicate: LogicalExpr::col("a").gt(LogicalExpr::lit(1)),
        input: Box::new(scan()),
    };
    let once = Optimizer::new().optimize(plan);
    let twice = Optimizer::new().optimize(once.clone());
    assert_eq!(once, twice, "optimizer must reach a fixpoint");
}
