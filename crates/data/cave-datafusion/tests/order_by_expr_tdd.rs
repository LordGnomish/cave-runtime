// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for `ORDER BY <expr>` where the sort key is not
//! a bare projected column — a scalar function of a column, or a column
//! that is *not* in the SELECT list.
//!
//! Upstream: `apache/datafusion` `datafusion-sql` LogicalPlanBuilder —
//! ORDER BY expressions are resolved against the *input* to the final
//! projection, so `SELECT b FROM t ORDER BY length(b)` and
//! `SELECT a FROM t ORDER BY b` both sort correctly. The cave-datafusion
//! MVP built the `Sort` node *above* the `Projection`, lowering the sort
//! key against the pre-projection schema while the executor saw
//! post-projection rows — so any key referencing a column whose
//! pre/post-projection index differed silently evaluated to NULL and the
//! rows came back in input order. Pins the corrected behavior (RED).

use cave_datafusion::row::{Row, Value};
use cave_datafusion::schema::{DataType, Field, SchemaRef, TableSchema};
use cave_datafusion::SessionContext;
use std::sync::Arc;

async fn ctx_ab() -> SessionContext {
    let ctx = SessionContext::new();
    let schema: SchemaRef = Arc::new(TableSchema::new(vec![
        Field::new("a", DataType::Int64, false),
        Field::new("b", DataType::Utf8, false),
    ]));
    // Insertion order is deliberately neither ascending nor descending by
    // length(b) nor by b, so a no-op sort is distinguishable from a real one.
    let rows = vec![
        Row::new(vec![Value::Int64(1), Value::Utf8("ccc".into())]),
        Row::new(vec![Value::Int64(2), Value::Utf8("a".into())]),
        Row::new(vec![Value::Int64(3), Value::Utf8("bb".into())]),
    ];
    ctx.register_mem_table("t", schema, rows).await.unwrap();
    ctx
}

fn col0(rows: &[Row]) -> Vec<Value> {
    rows.iter().map(|r| r.values[0].clone()).collect()
}

#[tokio::test]
async fn order_by_scalar_function_of_column_ascending() {
    let ctx = ctx_ab().await;
    let out = ctx
        .sql("SELECT a FROM t ORDER BY length(b)")
        .await
        .unwrap();
    // length: ccc=3, a=1, bb=2 → ascending a, bb, ccc → a-column 2, 3, 1.
    assert_eq!(
        col0(&out),
        vec![Value::Int64(2), Value::Int64(3), Value::Int64(1)]
    );
}

#[tokio::test]
async fn order_by_scalar_function_descending() {
    let ctx = ctx_ab().await;
    let out = ctx
        .sql("SELECT a FROM t ORDER BY length(b) DESC")
        .await
        .unwrap();
    // descending length: ccc(3), bb(2), a(1) → a-column 1, 3, 2.
    assert_eq!(
        col0(&out),
        vec![Value::Int64(1), Value::Int64(3), Value::Int64(2)]
    );
}

#[tokio::test]
async fn order_by_column_not_in_select_list() {
    let ctx = ctx_ab().await;
    // Classic: ORDER BY references a column absent from the projection.
    let out = ctx.sql("SELECT a FROM t ORDER BY b").await.unwrap();
    // b ascending: a, bb, ccc → a-column 2, 3, 1.
    assert_eq!(
        col0(&out),
        vec![Value::Int64(2), Value::Int64(3), Value::Int64(1)]
    );
    // Output projects only `a`, exactly one column.
    assert_eq!(out[0].values.len(), 1);
}

#[tokio::test]
async fn order_by_projected_column_still_works() {
    // Regression guard: ordering by a column that *is* selected must still
    // work after the Sort/Projection reorder.
    let ctx = ctx_ab().await;
    let out = ctx
        .sql("SELECT a FROM t ORDER BY a DESC")
        .await
        .unwrap();
    assert_eq!(
        col0(&out),
        vec![Value::Int64(3), Value::Int64(2), Value::Int64(1)]
    );
}
