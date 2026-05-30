// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for row-level scalar function invocation —
//! `PhysicalExpr::Call`, the lowered form of a `LogicalExpr::Function`
//! whose arguments are not all literals.
//!
//! Upstream: `apache/datafusion`
//! `datafusion-physical-expr/src/scalar_function.rs` (`ScalarFunctionExpr`).
//! There, a scalar UDF is wrapped in a `PhysicalExpr` that evaluates its
//! argument columns per batch and invokes the function. The cave-datafusion
//! MVP previously only folded scalar calls whose args were all constant
//! (at plan-lowering time); a `Column` argument errored. This pins the
//! `Call` variant + its per-row evaluation, and the end-to-end SQL path
//! `SELECT upper(b) FROM t`, before the variant exists (RED).

use cave_datafusion::functions::{FunctionRegistry, ScalarFnHandle};
use cave_datafusion::physical_expr::PhysicalExpr;
use cave_datafusion::row::{Row, Value};
use cave_datafusion::schema::{DataType, Field, SchemaRef, TableSchema};
use cave_datafusion::SessionContext;
use std::sync::Arc;

#[test]
fn physical_call_evaluates_function_per_row() {
    let reg = FunctionRegistry::new();
    let upper = reg.lookup_scalar("upper").expect("upper registered").fun.clone();
    let call = PhysicalExpr::Call {
        name: "upper".into(),
        fun: ScalarFnHandle(upper),
        args: vec![PhysicalExpr::Column { index: 0 }],
    };
    let row = Row::new(vec![Value::Utf8("hello".into())]);
    assert_eq!(call.evaluate(&row).unwrap(), Value::Utf8("HELLO".into()));
}

#[test]
fn physical_call_threads_multiple_args() {
    let reg = FunctionRegistry::new();
    let concat = reg.lookup_scalar("concat").expect("concat registered").fun.clone();
    let call = PhysicalExpr::Call {
        name: "concat".into(),
        fun: ScalarFnHandle(concat),
        args: vec![
            PhysicalExpr::Column { index: 0 },
            PhysicalExpr::Literal {
                value: Value::Utf8("!".into()),
            },
        ],
    };
    let row = Row::new(vec![Value::Utf8("hi".into())]);
    assert_eq!(call.evaluate(&row).unwrap(), Value::Utf8("hi!".into()));
}

#[tokio::test]
async fn sql_scalar_function_over_a_column() {
    let ctx = SessionContext::new();
    let schema: SchemaRef = Arc::new(TableSchema::new(vec![Field::new(
        "b",
        DataType::Utf8,
        true,
    )]));
    let rows = vec![
        Row::new(vec![Value::Utf8("alpha".into())]),
        Row::new(vec![Value::Utf8("beta".into())]),
    ];
    ctx.register_mem_table("t", schema, rows).await.unwrap();

    let out = ctx.sql("SELECT upper(b) FROM t").await.unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].values[0], Value::Utf8("ALPHA".into()));
    assert_eq!(out[1].values[0], Value::Utf8("BETA".into()));
}
