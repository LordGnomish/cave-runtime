// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ExecutionPlan — physical operators that execute against a RecordBatch.
//!
//! Mirrors apache/datafusion datafusion-physical-plan/src/{projection,filter,
//! limit,aggregates}.rs (subset). Each operator takes an input `RecordBatch`
//! and returns a new `RecordBatch`.

use crate::engine::datafusion::batch::{RecordBatch, Value};
use crate::engine::datafusion::error::{DataFusionError, DfResult};
use crate::engine::datafusion::expr::Expr;
use crate::engine::datafusion::logical_plan::{AggregateExpr, AggregateFunc, LogicalPlan};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionPlan {
    Source(RecordBatch),
    Projection {
        input: Box<ExecutionPlan>,
        columns: Vec<String>,
    },
    Filter {
        input: Box<ExecutionPlan>,
        predicate: Expr,
    },
    Limit {
        input: Box<ExecutionPlan>,
        skip: usize,
        fetch: Option<usize>,
    },
    Aggregate {
        input: Box<ExecutionPlan>,
        group_by: Vec<String>,
        aggregates: Vec<AggregateExpr>,
    },
}

impl ExecutionPlan {
    /// Run this plan and return the result batch.
    pub fn execute(&self) -> DfResult<RecordBatch> {
        match self {
            ExecutionPlan::Source(b) => Ok(b.clone()),
            ExecutionPlan::Projection { input, columns } => {
                let in_batch = input.execute()?;
                project(&in_batch, columns)
            }
            ExecutionPlan::Filter { input, predicate } => {
                let in_batch = input.execute()?;
                filter(&in_batch, predicate)
            }
            ExecutionPlan::Limit { input, skip, fetch } => {
                let in_batch = input.execute()?;
                limit(&in_batch, *skip, *fetch)
            }
            ExecutionPlan::Aggregate {
                input,
                group_by,
                aggregates,
            } => {
                let in_batch = input.execute()?;
                aggregate(&in_batch, group_by, aggregates)
            }
        }
    }
}

fn project(batch: &RecordBatch, columns: &[String]) -> DfResult<RecordBatch> {
    if columns.is_empty() {
        return Err(DataFusionError::Plan(
            "projection must select ≥ 1 column".into(),
        ));
    }
    let indices: Vec<usize> = columns
        .iter()
        .map(|c| batch.column_index(c))
        .collect::<DfResult<_>>()?;
    let rows: Vec<Vec<Value>> = batch
        .rows
        .iter()
        .map(|row| indices.iter().map(|&i| row[i].clone()).collect())
        .collect();
    Ok(RecordBatch {
        columns: columns.to_vec(),
        rows,
        tenant_id: batch.tenant_id.clone(),
    })
}

fn filter(batch: &RecordBatch, predicate: &Expr) -> DfResult<RecordBatch> {
    let mut kept = Vec::new();
    for row in &batch.rows {
        let v = predicate.evaluate(batch, row)?;
        match v {
            // SQL: only Bool::true keeps the row; null/false discards
            Value::Bool(true) => kept.push(row.clone()),
            Value::Bool(false) | Value::Null => {}
            other => {
                return Err(DataFusionError::TypeMismatch(format!(
                    "filter predicate must be bool, got {}",
                    other.type_name()
                )));
            }
        }
    }
    Ok(RecordBatch {
        columns: batch.columns.clone(),
        rows: kept,
        tenant_id: batch.tenant_id.clone(),
    })
}

fn limit(batch: &RecordBatch, skip: usize, fetch: Option<usize>) -> DfResult<RecordBatch> {
    let start = skip.min(batch.rows.len());
    let end = match fetch {
        Some(n) => (start + n).min(batch.rows.len()),
        None => batch.rows.len(),
    };
    let rows = batch.rows[start..end].to_vec();
    Ok(RecordBatch {
        columns: batch.columns.clone(),
        rows,
        tenant_id: batch.tenant_id.clone(),
    })
}

fn aggregate(
    batch: &RecordBatch,
    group_by: &[String],
    aggregates: &[AggregateExpr],
) -> DfResult<RecordBatch> {
    if aggregates.is_empty() {
        return Err(DataFusionError::Plan(
            "aggregate must compute ≥ 1 aggregate".into(),
        ));
    }
    let group_indices: Vec<usize> = group_by
        .iter()
        .map(|c| batch.column_index(c))
        .collect::<DfResult<_>>()?;
    // group key → rows
    let mut groups: BTreeMap<Vec<String>, Vec<&Vec<Value>>> = BTreeMap::new();
    for row in &batch.rows {
        let key: Vec<String> = group_indices
            .iter()
            .map(|&i| value_key(&row[i]))
            .collect();
        groups.entry(key).or_default().push(row);
    }
    let mut out_rows: Vec<Vec<Value>> = Vec::new();
    let mut out_cols: Vec<String> = group_by.to_vec();
    for a in aggregates {
        out_cols.push(a.output_name.clone());
    }
    // sort by group key for determinism (BTreeMap already does)
    for (key, rows) in &groups {
        let mut out = Vec::with_capacity(out_cols.len());
        for k in key {
            out.push(decode_key(k));
        }
        for a in aggregates {
            let v = compute_agg(batch, rows, a)?;
            out.push(v);
        }
        out_rows.push(out);
    }
    // empty input + no group_by → still produce one row (SQL semantics for
    // bare aggregates over empty input)
    if out_rows.is_empty() && group_by.is_empty() {
        let mut out = Vec::with_capacity(out_cols.len());
        for a in aggregates {
            out.push(empty_agg_value(a.func));
        }
        out_rows.push(out);
    }
    Ok(RecordBatch {
        columns: out_cols,
        rows: out_rows,
        tenant_id: batch.tenant_id.clone(),
    })
}

fn value_key(v: &Value) -> String {
    match v {
        Value::Null => "\0null".into(),
        Value::Bool(b) => format!("b:{}", b),
        Value::Int64(i) => format!("i:{}", i),
        Value::Float64(f) => format!("f:{}", f),
        Value::Utf8(s) => format!("s:{}", s),
    }
}

fn decode_key(k: &str) -> Value {
    if k == "\0null" {
        return Value::Null;
    }
    if let Some(rest) = k.strip_prefix("b:") {
        return Value::Bool(rest == "true");
    }
    if let Some(rest) = k.strip_prefix("i:") {
        return Value::Int64(rest.parse().unwrap_or(0));
    }
    if let Some(rest) = k.strip_prefix("f:") {
        return Value::Float64(rest.parse().unwrap_or(0.0));
    }
    if let Some(rest) = k.strip_prefix("s:") {
        return Value::Utf8(rest.to_string());
    }
    Value::Null
}

fn compute_agg(
    batch: &RecordBatch,
    rows: &[&Vec<Value>],
    a: &AggregateExpr,
) -> DfResult<Value> {
    match a.func {
        AggregateFunc::Count => {
            // COUNT(*) → all rows; COUNT(col) → non-null rows of col
            let n = if let Some(col) = &a.column {
                let idx = batch.column_index(col)?;
                rows.iter().filter(|r| !r[idx].is_null()).count()
            } else {
                rows.len()
            };
            Ok(Value::Int64(n as i64))
        }
        AggregateFunc::Sum => {
            let col = a.column.as_ref().ok_or_else(|| {
                DataFusionError::Plan("SUM requires a column".into())
            })?;
            let idx = batch.column_index(col)?;
            let mut total: i64 = 0;
            for r in rows {
                if let Some(v) = r[idx].as_int64() {
                    total = total.saturating_add(v);
                }
            }
            Ok(Value::Int64(total))
        }
        AggregateFunc::Min => {
            let col = a.column.as_ref().ok_or_else(|| {
                DataFusionError::Plan("MIN requires a column".into())
            })?;
            let idx = batch.column_index(col)?;
            let mut m: Option<i64> = None;
            for r in rows {
                if let Some(v) = r[idx].as_int64() {
                    m = Some(m.map_or(v, |c| c.min(v)));
                }
            }
            Ok(m.map(Value::Int64).unwrap_or(Value::Null))
        }
        AggregateFunc::Max => {
            let col = a.column.as_ref().ok_or_else(|| {
                DataFusionError::Plan("MAX requires a column".into())
            })?;
            let idx = batch.column_index(col)?;
            let mut m: Option<i64> = None;
            for r in rows {
                if let Some(v) = r[idx].as_int64() {
                    m = Some(m.map_or(v, |c| c.max(v)));
                }
            }
            Ok(m.map(Value::Int64).unwrap_or(Value::Null))
        }
        AggregateFunc::Avg => {
            let col = a.column.as_ref().ok_or_else(|| {
                DataFusionError::Plan("AVG requires a column".into())
            })?;
            let idx = batch.column_index(col)?;
            let mut total: i64 = 0;
            let mut n: i64 = 0;
            for r in rows {
                if let Some(v) = r[idx].as_int64() {
                    total = total.saturating_add(v);
                    n += 1;
                }
            }
            if n == 0 {
                Ok(Value::Null)
            } else {
                Ok(Value::Float64(total as f64 / n as f64))
            }
        }
    }
}

fn empty_agg_value(f: AggregateFunc) -> Value {
    match f {
        AggregateFunc::Count => Value::Int64(0),
        // SQL: SUM/MIN/MAX/AVG over empty input return NULL
        _ => Value::Null,
    }
}

/// Compile a `LogicalPlan` against a source batch into an `ExecutionPlan`.
pub fn compile(plan: &LogicalPlan, source: RecordBatch) -> DfResult<ExecutionPlan> {
    plan.validate()?;
    Ok(compile_inner(plan, source))
}

fn compile_inner(plan: &LogicalPlan, source: RecordBatch) -> ExecutionPlan {
    match plan {
        LogicalPlan::Scan { .. } => ExecutionPlan::Source(source),
        LogicalPlan::Projection { input, columns } => ExecutionPlan::Projection {
            input: Box::new(compile_inner(input, source)),
            columns: columns.clone(),
        },
        LogicalPlan::Filter { input, predicate } => ExecutionPlan::Filter {
            input: Box::new(compile_inner(input, source)),
            predicate: predicate.clone(),
        },
        LogicalPlan::Limit { input, skip, fetch } => ExecutionPlan::Limit {
            input: Box::new(compile_inner(input, source)),
            skip: *skip,
            fetch: *fetch,
        },
        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => ExecutionPlan::Aggregate {
            input: Box::new(compile_inner(input, source)),
            group_by: group_by.clone(),
            aggregates: aggregates.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::datafusion::batch::{RecordBatch, Value};

    fn employees() -> RecordBatch {
        RecordBatch::new(
            vec!["id".into(), "dept".into(), "salary".into()],
            vec![
                vec![Value::Int64(1), Value::Utf8("eng".into()), Value::Int64(100)],
                vec![Value::Int64(2), Value::Utf8("eng".into()), Value::Int64(120)],
                vec![Value::Int64(3), Value::Utf8("sales".into()), Value::Int64(80)],
                vec![Value::Int64(4), Value::Utf8("sales".into()), Value::Int64(90)],
                vec![Value::Int64(5), Value::Utf8("ops".into()), Value::Int64(70)],
            ],
        )
        .unwrap()
    }

    fn src() -> ExecutionPlan {
        ExecutionPlan::Source(employees())
    }

    // ── Source ────────────────────────────────────────────────────────────────

    #[test]
    fn source_returns_input_unchanged() {
        let out = src().execute().unwrap();
        assert_eq!(out, employees());
    }

    // ── Projection ────────────────────────────────────────────────────────────

    #[test]
    fn projection_keeps_only_requested_columns() {
        let p = ExecutionPlan::Projection {
            input: Box::new(src()),
            columns: vec!["id".into(), "salary".into()],
        };
        let out = p.execute().unwrap();
        assert_eq!(out.columns, vec!["id".to_string(), "salary".to_string()]);
        assert_eq!(out.rows[0], vec![Value::Int64(1), Value::Int64(100)]);
    }

    #[test]
    fn projection_reorder_columns() {
        let p = ExecutionPlan::Projection {
            input: Box::new(src()),
            columns: vec!["dept".into(), "id".into()],
        };
        let out = p.execute().unwrap();
        assert_eq!(out.rows[0], vec![Value::Utf8("eng".into()), Value::Int64(1)]);
    }

    #[test]
    fn projection_unknown_column_err() {
        let p = ExecutionPlan::Projection {
            input: Box::new(src()),
            columns: vec!["nope".into()],
        };
        assert!(p.execute().is_err());
    }

    #[test]
    fn projection_empty_columns_err() {
        let p = ExecutionPlan::Projection {
            input: Box::new(src()),
            columns: vec![],
        };
        assert!(p.execute().is_err());
    }

    #[test]
    fn projection_preserves_tenant() {
        let mut b = employees();
        b.tenant_id = "acme".into();
        let p = ExecutionPlan::Projection {
            input: Box::new(ExecutionPlan::Source(b)),
            columns: vec!["id".into()],
        };
        let out = p.execute().unwrap();
        assert_eq!(out.tenant_id, "acme");
    }

    // ── Filter ────────────────────────────────────────────────────────────────

    #[test]
    fn filter_keeps_matching_rows() {
        let p = ExecutionPlan::Filter {
            input: Box::new(src()),
            predicate: Expr::col("dept").eq(Expr::lit(Value::Utf8("eng".into()))),
        };
        let out = p.execute().unwrap();
        assert_eq!(out.num_rows(), 2);
    }

    #[test]
    fn filter_keeps_zero_rows() {
        let p = ExecutionPlan::Filter {
            input: Box::new(src()),
            predicate: Expr::col("salary").gt(Expr::lit(Value::Int64(1000))),
        };
        let out = p.execute().unwrap();
        assert_eq!(out.num_rows(), 0);
    }

    #[test]
    fn filter_keeps_all_rows() {
        let p = ExecutionPlan::Filter {
            input: Box::new(src()),
            predicate: Expr::lit(Value::Bool(true)),
        };
        let out = p.execute().unwrap();
        assert_eq!(out.num_rows(), 5);
    }

    #[test]
    fn filter_null_drops_row() {
        // SQL: predicate = null → row is dropped (only true keeps)
        let b = RecordBatch::new(
            vec!["x".into()],
            vec![vec![Value::Null], vec![Value::Int64(1)], vec![Value::Int64(2)]],
        )
        .unwrap();
        let p = ExecutionPlan::Filter {
            input: Box::new(ExecutionPlan::Source(b)),
            predicate: Expr::col("x").gt(Expr::lit(Value::Int64(0))),
        };
        let out = p.execute().unwrap();
        assert_eq!(out.num_rows(), 2);
    }

    #[test]
    fn filter_non_bool_predicate_err() {
        let p = ExecutionPlan::Filter {
            input: Box::new(src()),
            predicate: Expr::lit(Value::Int64(1)),
        };
        assert!(p.execute().is_err());
    }

    // ── Limit ─────────────────────────────────────────────────────────────────

    #[test]
    fn limit_fetch_only() {
        let p = ExecutionPlan::Limit {
            input: Box::new(src()),
            skip: 0,
            fetch: Some(3),
        };
        let out = p.execute().unwrap();
        assert_eq!(out.num_rows(), 3);
    }

    #[test]
    fn limit_skip_and_fetch() {
        let p = ExecutionPlan::Limit {
            input: Box::new(src()),
            skip: 2,
            fetch: Some(2),
        };
        let out = p.execute().unwrap();
        assert_eq!(out.num_rows(), 2);
        assert_eq!(out.rows[0][0], Value::Int64(3));
    }

    #[test]
    fn limit_skip_past_end_returns_empty() {
        let p = ExecutionPlan::Limit {
            input: Box::new(src()),
            skip: 999,
            fetch: Some(10),
        };
        assert_eq!(p.execute().unwrap().num_rows(), 0);
    }

    #[test]
    fn limit_no_fetch_returns_remaining() {
        let p = ExecutionPlan::Limit {
            input: Box::new(src()),
            skip: 2,
            fetch: None,
        };
        assert_eq!(p.execute().unwrap().num_rows(), 3);
    }

    #[test]
    fn limit_fetch_more_than_available_caps() {
        let p = ExecutionPlan::Limit {
            input: Box::new(src()),
            skip: 0,
            fetch: Some(100),
        };
        assert_eq!(p.execute().unwrap().num_rows(), 5);
    }

    // ── Aggregate ─────────────────────────────────────────────────────────────

    #[test]
    fn aggregate_count_star_no_group() {
        let p = ExecutionPlan::Aggregate {
            input: Box::new(src()),
            group_by: vec![],
            aggregates: vec![AggregateExpr {
                func: AggregateFunc::Count,
                column: None,
                output_name: "n".into(),
            }],
        };
        let out = p.execute().unwrap();
        assert_eq!(out.num_rows(), 1);
        assert_eq!(out.rows[0][0], Value::Int64(5));
    }

    #[test]
    fn aggregate_sum_no_group() {
        let p = ExecutionPlan::Aggregate {
            input: Box::new(src()),
            group_by: vec![],
            aggregates: vec![AggregateExpr {
                func: AggregateFunc::Sum,
                column: Some("salary".into()),
                output_name: "total".into(),
            }],
        };
        let out = p.execute().unwrap();
        assert_eq!(out.rows[0][0], Value::Int64(100 + 120 + 80 + 90 + 70));
    }

    #[test]
    fn aggregate_group_by_dept_count() {
        let p = ExecutionPlan::Aggregate {
            input: Box::new(src()),
            group_by: vec!["dept".into()],
            aggregates: vec![AggregateExpr {
                func: AggregateFunc::Count,
                column: None,
                output_name: "n".into(),
            }],
        };
        let out = p.execute().unwrap();
        assert_eq!(out.num_rows(), 3); // eng, ops, sales
        assert_eq!(out.columns, vec!["dept".to_string(), "n".to_string()]);
    }

    #[test]
    fn aggregate_group_by_dept_sum_salary() {
        let p = ExecutionPlan::Aggregate {
            input: Box::new(src()),
            group_by: vec!["dept".into()],
            aggregates: vec![AggregateExpr {
                func: AggregateFunc::Sum,
                column: Some("salary".into()),
                output_name: "total".into(),
            }],
        };
        let out = p.execute().unwrap();
        // BTreeMap → key sorted: eng (220), ops (70), sales (170)
        assert_eq!(out.rows[0][1], Value::Int64(220));
        assert_eq!(out.rows[1][1], Value::Int64(70));
        assert_eq!(out.rows[2][1], Value::Int64(170));
    }

    #[test]
    fn aggregate_min_max() {
        let p = ExecutionPlan::Aggregate {
            input: Box::new(src()),
            group_by: vec![],
            aggregates: vec![
                AggregateExpr {
                    func: AggregateFunc::Min,
                    column: Some("salary".into()),
                    output_name: "min_s".into(),
                },
                AggregateExpr {
                    func: AggregateFunc::Max,
                    column: Some("salary".into()),
                    output_name: "max_s".into(),
                },
            ],
        };
        let out = p.execute().unwrap();
        assert_eq!(out.rows[0][0], Value::Int64(70));
        assert_eq!(out.rows[0][1], Value::Int64(120));
    }

    #[test]
    fn aggregate_avg_returns_float() {
        let p = ExecutionPlan::Aggregate {
            input: Box::new(src()),
            group_by: vec![],
            aggregates: vec![AggregateExpr {
                func: AggregateFunc::Avg,
                column: Some("salary".into()),
                output_name: "avg_s".into(),
            }],
        };
        let out = p.execute().unwrap();
        let avg = (100 + 120 + 80 + 90 + 70) as f64 / 5.0;
        assert_eq!(out.rows[0][0], Value::Float64(avg));
    }

    #[test]
    fn aggregate_empty_input_no_group_returns_one_row_zero_count() {
        // citation: SQL standard — aggregate on empty input with no group
        // returns one row (COUNT=0, SUM/MIN/MAX/AVG=NULL)
        let empty = RecordBatch::empty(vec!["x".into()]);
        let p = ExecutionPlan::Aggregate {
            input: Box::new(ExecutionPlan::Source(empty)),
            group_by: vec![],
            aggregates: vec![AggregateExpr {
                func: AggregateFunc::Count,
                column: None,
                output_name: "n".into(),
            }],
        };
        let out = p.execute().unwrap();
        assert_eq!(out.num_rows(), 1);
        assert_eq!(out.rows[0][0], Value::Int64(0));
    }

    #[test]
    fn aggregate_empty_input_no_group_sum_returns_null() {
        let empty = RecordBatch::empty(vec!["x".into()]);
        let p = ExecutionPlan::Aggregate {
            input: Box::new(ExecutionPlan::Source(empty)),
            group_by: vec![],
            aggregates: vec![AggregateExpr {
                func: AggregateFunc::Sum,
                column: Some("x".into()),
                output_name: "s".into(),
            }],
        };
        let out = p.execute().unwrap();
        assert_eq!(out.rows[0][0], Value::Null);
    }

    #[test]
    fn aggregate_count_col_excludes_nulls() {
        let b = RecordBatch::new(
            vec!["x".into()],
            vec![
                vec![Value::Int64(1)],
                vec![Value::Null],
                vec![Value::Int64(2)],
            ],
        )
        .unwrap();
        let p = ExecutionPlan::Aggregate {
            input: Box::new(ExecutionPlan::Source(b)),
            group_by: vec![],
            aggregates: vec![AggregateExpr {
                func: AggregateFunc::Count,
                column: Some("x".into()),
                output_name: "n".into(),
            }],
        };
        assert_eq!(p.execute().unwrap().rows[0][0], Value::Int64(2));
    }

    #[test]
    fn aggregate_unknown_group_col_err() {
        let p = ExecutionPlan::Aggregate {
            input: Box::new(src()),
            group_by: vec!["nope".into()],
            aggregates: vec![AggregateExpr {
                func: AggregateFunc::Count,
                column: None,
                output_name: "n".into(),
            }],
        };
        assert!(p.execute().is_err());
    }

    #[test]
    fn aggregate_no_aggregates_err() {
        let p = ExecutionPlan::Aggregate {
            input: Box::new(src()),
            group_by: vec!["dept".into()],
            aggregates: vec![],
        };
        assert!(p.execute().is_err());
    }

    // ── compile() ─────────────────────────────────────────────────────────────

    #[test]
    fn compile_round_trip_filter_projection_limit() {
        let plan = LogicalPlan::scan("emp")
            .filter(Expr::col("dept").eq(Expr::lit(Value::Utf8("eng".into()))))
            .project(vec!["salary".into()])
            .limit(0, Some(5));
        let exec = compile(&plan, employees()).unwrap();
        let out = exec.execute().unwrap();
        assert_eq!(out.columns, vec!["salary".to_string()]);
        assert_eq!(out.num_rows(), 2);
    }

    #[test]
    fn compile_invalid_plan_err() {
        let plan = LogicalPlan::scan("emp").project(vec![]);
        assert!(compile(&plan, employees()).is_err());
    }

    #[test]
    fn compile_aggregate_count_star() {
        let plan = LogicalPlan::scan("emp").aggregate(
            vec![],
            vec![AggregateExpr {
                func: AggregateFunc::Count,
                column: None,
                output_name: "n".into(),
            }],
        );
        let out = compile(&plan, employees()).unwrap().execute().unwrap();
        assert_eq!(out.rows[0][0], Value::Int64(5));
    }
}
