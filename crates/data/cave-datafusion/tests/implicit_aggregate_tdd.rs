// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for implicit (whole-table) aggregation — an
//! aggregate in the SELECT list with no GROUP BY clause.
//!
//! Upstream: `apache/datafusion` `datafusion-sql` — a SELECT whose
//! projection contains an aggregate function but no GROUP BY aggregates
//! the entire input as a single group and returns exactly one row
//! (`SELECT count(a) FROM t`, `SELECT sum(a) FROM t`). The cave-datafusion
//! MVP only built an Aggregate plan node when GROUP BY was present, so a
//! bare `sum(...)`/`count(...)` was mistaken for a scalar function and
//! errored with `unknown scalar function: sum`. Pins the single-group
//! semantics, nested scalar-in-aggregate, and the empty-input case (RED).

use cave_datafusion::row::{Row, Value};
use cave_datafusion::schema::{DataType, Field, SchemaRef, TableSchema};
use cave_datafusion::SessionContext;
use std::sync::Arc;

fn schema_a() -> SchemaRef {
    Arc::new(TableSchema::new(vec![Field::new("a", DataType::Int64, true)]))
}

async fn ctx_with(rows: Vec<Row>) -> SessionContext {
    let ctx = SessionContext::new();
    ctx.register_mem_table("t", schema_a(), rows).await.unwrap();
    ctx
}

#[tokio::test]
async fn count_without_group_by() {
    let ctx = ctx_with(vec![
        Row::new(vec![Value::Int64(-5)]),
        Row::new(vec![Value::Int64(3)]),
        Row::new(vec![Value::Null]),
    ])
    .await;
    let out = ctx.sql("SELECT count(a) FROM t").await.unwrap();
    assert_eq!(out.len(), 1);
    // count ignores NULLs → 2 non-null values.
    assert_eq!(out[0].values[0], Value::Int64(2));
}

#[tokio::test]
async fn sum_without_group_by_stays_int64() {
    let ctx = ctx_with(vec![
        Row::new(vec![Value::Int64(-5)]),
        Row::new(vec![Value::Int64(3)]),
    ])
    .await;
    let out = ctx.sql("SELECT sum(a) FROM t").await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].values[0], Value::Int64(-2));
}

#[tokio::test]
async fn nested_scalar_inside_implicit_aggregate() {
    let ctx = ctx_with(vec![
        Row::new(vec![Value::Int64(-5)]),
        Row::new(vec![Value::Int64(3)]),
    ])
    .await;
    // sum(abs(a)) = 5 + 3 = 8.
    let out = ctx.sql("SELECT sum(abs(a)) FROM t").await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].values[0], Value::Int64(8));
}

#[tokio::test]
async fn count_over_empty_input_is_one_row_zero() {
    // Implicit aggregation always emits exactly one row, even over an
    // empty input — count → 0.
    let ctx = ctx_with(vec![]).await;
    let out = ctx.sql("SELECT count(a) FROM t").await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].values[0], Value::Int64(0));
}
