// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Coverage backing the `scalar_function` partial→mapped reclassification.
//!
//! Upstream: `apache/datafusion`
//! `datafusion-physical-expr/src/scalar_function.rs` (`ScalarFunctionExpr`).
//! The partial's only deferred claim was *row-level* scalar-function
//! invocation (a non-literal argument), which now lowers to
//! `PhysicalExpr::Call` and evaluates per row. These tests pin that the
//! Call path works in every expression position the MVP exposes — a
//! projection, a nested call, and a WHERE predicate — so the mapping is
//! complete, not just shape. (ORDER BY of a scalar function is covered
//! by `order_by_expr_tdd.rs`; the core Call variant by `scalar_call_tdd.rs`.)

use cave_datafusion::row::{Row, Value};
use cave_datafusion::schema::{DataType, Field, SchemaRef, TableSchema};
use cave_datafusion::SessionContext;
use std::sync::Arc;

async fn ctx_ab() -> SessionContext {
    let ctx = SessionContext::new();
    let schema: SchemaRef = Arc::new(TableSchema::new(vec![
        Field::new("a", DataType::Int64, true),
        Field::new("b", DataType::Utf8, true),
    ]));
    let rows = vec![
        Row::new(vec![Value::Int64(-5), Value::Utf8("alpha".into())]),
        Row::new(vec![Value::Int64(3), Value::Utf8("be".into())]),
    ];
    ctx.register_mem_table("t", schema, rows).await.unwrap();
    ctx
}

#[tokio::test]
async fn nested_scalar_call_over_a_column() {
    let ctx = ctx_ab().await;
    let out = ctx.sql("SELECT upper(concat(b, '!')) FROM t").await.unwrap();
    assert_eq!(out[0].values[0], Value::Utf8("ALPHA!".into()));
    assert_eq!(out[1].values[0], Value::Utf8("BE!".into()));
}

#[tokio::test]
async fn scalar_call_in_where_predicate() {
    let ctx = ctx_ab().await;
    // length(b): alpha=5, be=2 → only the first row passes `> 3`.
    let out = ctx.sql("SELECT b FROM t WHERE length(b) > 3").await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].values[0], Value::Utf8("alpha".into()));
}

#[tokio::test]
async fn scalar_call_over_numeric_column() {
    let ctx = ctx_ab().await;
    // abs over a column, row-level.
    let out = ctx.sql("SELECT abs(a) FROM t").await.unwrap();
    assert_eq!(out[0].values[0], Value::Int64(5));
    assert_eq!(out[1].values[0], Value::Int64(3));
}
