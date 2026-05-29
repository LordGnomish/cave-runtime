// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

//! Strict-TDD regression test: SUM over Int64 columns must preserve Int64.
//!
//! DataFusion preserves the integer type for integer SUM (the accumulator does
//! not coerce Int64 inputs to Float64). This test pins that behaviour for the
//! cave-datafusion physical aggregate executor.

use std::sync::Arc;

use cave_datafusion::functions::AggregateKind;
use cave_datafusion::physical_expr::PhysicalExpr;
use cave_datafusion::physical_plan::{ExecutionPlan, PhysicalPlan};
use cave_datafusion::row::{Row, Value};
use cave_datafusion::schema::{DataType, Field, SchemaRef, TableSchema};

/// Build a single-column Int64 input plan, aggregate SUM over it, and run.
fn sum_over_int64(values: Vec<i64>) -> Value {
    let schema: SchemaRef = Arc::new(TableSchema::new(vec![Field::new(
        "v",
        DataType::Int64,
        false,
    )]));
    let rows: Vec<Row> = values
        .into_iter()
        .map(|n| Row::new(vec![Value::Int64(n)]))
        .collect();

    let scan = PhysicalPlan::InMemoryScan {
        rows,
        schema: schema.clone(),
    };

    let agg = PhysicalPlan::Aggregate {
        group_by: vec![],
        aggr: vec![(AggregateKind::Sum, PhysicalExpr::Column { index: 0 })],
        input: Box::new(scan),
        schema,
    };

    let out = agg.execute().expect("aggregate execute");
    assert_eq!(out.len(), 1, "expected a single aggregate row");
    out[0].values[0].clone()
}

#[test]
fn sum_of_int64_column_stays_int64() {
    let result = sum_over_int64(vec![10, 20]);
    assert_eq!(
        result,
        Value::Int64(30),
        "SUM over Int64 inputs must stay Int64, not coerce to Float64"
    );
}

#[test]
fn sum_of_int64_column_value_is_correct() {
    assert_eq!(sum_over_int64(vec![1, 2, 3, 4]), Value::Int64(10));
}
