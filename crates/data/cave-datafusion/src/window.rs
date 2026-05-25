// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Window functions — `crates/datafusion-functions-window/`.
//!
//! Implements the canonical window functions over a sorted partition:
//! `ROW_NUMBER`, `RANK`, `DENSE_RANK`, `LAG`, `LEAD`, `FIRST_VALUE`,
//! `LAST_VALUE`, `NTILE`. The MVP runs over column-oriented `&[Value]`
//! slices — callers project the target column out of their row buffer
//! before invoking [`evaluate`]. Integrating into the physical plan
//! happens once an `ExecutionPlan::WindowExec` node lands alongside
//! `AggregateExec`.

use crate::row::Value;

/// One frame inside a partition. `start`/`end` are half-open indices
/// into the partition's row slice, matching upstream's `Range` shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub start: usize,
    pub end: usize,
}

/// Discriminator over the window functions cave-datafusion ships.
/// `column` here is the field the function operates over; the
/// physical planner is responsible for projecting that column out of
/// the row buffer before invoking [`evaluate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowFunction {
    RowNumber,
    Rank,
    DenseRank,
    Lag { offset: usize },
    Lead { offset: usize },
    FirstValue,
    LastValue,
    Ntile { buckets: usize },
}

impl WindowFunction {
    pub fn name(&self) -> &'static str {
        match self {
            WindowFunction::RowNumber => "row_number",
            WindowFunction::Rank => "rank",
            WindowFunction::DenseRank => "dense_rank",
            WindowFunction::Lag { .. } => "lag",
            WindowFunction::Lead { .. } => "lead",
            WindowFunction::FirstValue => "first_value",
            WindowFunction::LastValue => "last_value",
            WindowFunction::Ntile { .. } => "ntile",
        }
    }
}

/// Evaluate `f` over a partition's `values` column. `order_keys[i]`
/// is the value of the ORDER-BY column at row `i` (used by `Rank` and
/// `DenseRank` to detect ties). For functions that don't read the
/// data column (`RowNumber`, `Rank`, `DenseRank`, `Ntile`) the
/// `values` slice's length is the only thing that matters.
pub fn evaluate(
    f: &WindowFunction,
    values: &[Value],
    order_keys: &[Value],
) -> Result<Vec<Value>, String> {
    assert_eq!(
        values.len(),
        order_keys.len(),
        "values/order_keys shape mismatch"
    );
    let n = values.len();
    let out: Vec<Value> = match f {
        WindowFunction::RowNumber => (1..=n as i64).map(Value::Int64).collect(),
        WindowFunction::Rank => rank(order_keys, /*dense=*/ false),
        WindowFunction::DenseRank => rank(order_keys, /*dense=*/ true),
        WindowFunction::Lag { offset } => lag_or_lead(values, *offset, /*lag=*/ true),
        WindowFunction::Lead { offset } => lag_or_lead(values, *offset, /*lag=*/ false),
        WindowFunction::FirstValue => {
            let first = values.first().cloned().unwrap_or(Value::Null);
            vec![first; n]
        }
        WindowFunction::LastValue => {
            let last = values.last().cloned().unwrap_or(Value::Null);
            vec![last; n]
        }
        WindowFunction::Ntile { buckets } => ntile(n, *buckets)?,
    };
    Ok(out)
}

fn rank(order_keys: &[Value], dense: bool) -> Vec<Value> {
    let mut out = Vec::with_capacity(order_keys.len());
    let mut last: Option<&Value> = None;
    let mut current_rank: i64 = 0;
    let mut counted: i64 = 0;
    for v in order_keys {
        counted += 1;
        let same = last.map(|prev| prev == v).unwrap_or(false);
        if !same {
            current_rank = if dense { current_rank + 1 } else { counted };
        }
        out.push(Value::Int64(current_rank));
        last = Some(v);
    }
    out
}

fn lag_or_lead(values: &[Value], offset: usize, lag: bool) -> Vec<Value> {
    let n = values.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let neighbour: Option<usize> = if lag {
            i.checked_sub(offset)
        } else {
            let j = i + offset;
            if j < n { Some(j) } else { None }
        };
        let v = neighbour
            .map(|k| values[k].clone())
            .unwrap_or(Value::Null);
        out.push(v);
    }
    out
}

fn ntile(n: usize, buckets: usize) -> Result<Vec<Value>, String> {
    if buckets == 0 {
        return Err("ntile bucket count must be > 0".into());
    }
    if n == 0 {
        return Ok(Vec::new());
    }
    let base = n / buckets;
    let extra = n % buckets;
    let mut out = Vec::with_capacity(n);
    let mut bucket: i64 = 1;
    let mut consumed_in_bucket: usize = 0;
    let mut bucket_size = base + if extra > 0 { 1 } else { 0 };
    let mut buckets_with_extra_used = if extra > 0 { 1 } else { 0 };
    for _ in 0..n {
        if consumed_in_bucket == bucket_size {
            bucket += 1;
            consumed_in_bucket = 0;
            bucket_size = base + if buckets_with_extra_used < extra { 1 } else { 0 };
            if buckets_with_extra_used < extra {
                buckets_with_extra_used += 1;
            }
        }
        out.push(Value::Int64(bucket));
        consumed_in_bucket += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(ks: &[i64]) -> Vec<Value> {
        ks.iter().map(|k| Value::Int64(*k)).collect()
    }

    #[test]
    fn row_number_enumerates_sequentially() {
        let out = evaluate(&WindowFunction::RowNumber, &vals(&[10, 20, 30]), &vals(&[10, 20, 30]))
            .unwrap();
        assert_eq!(out, vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)]);
    }

    #[test]
    fn rank_ties_share_rank_then_skip() {
        let out = evaluate(
            &WindowFunction::Rank,
            &vals(&[10, 10, 20, 30]),
            &vals(&[10, 10, 20, 30]),
        )
        .unwrap();
        assert_eq!(
            out,
            vec![
                Value::Int64(1),
                Value::Int64(1),
                Value::Int64(3),
                Value::Int64(4)
            ]
        );
    }

    #[test]
    fn dense_rank_ties_share_rank_no_skip() {
        let out = evaluate(
            &WindowFunction::DenseRank,
            &vals(&[10, 10, 20, 30]),
            &vals(&[10, 10, 20, 30]),
        )
        .unwrap();
        assert_eq!(
            out,
            vec![
                Value::Int64(1),
                Value::Int64(1),
                Value::Int64(2),
                Value::Int64(3)
            ]
        );
    }

    #[test]
    fn lag_returns_null_before_window_start() {
        let out = evaluate(
            &WindowFunction::Lag { offset: 1 },
            &vals(&[10, 20, 30]),
            &vals(&[10, 20, 30]),
        )
        .unwrap();
        assert_eq!(out, vec![Value::Null, Value::Int64(10), Value::Int64(20)]);
    }

    #[test]
    fn lead_returns_null_past_window_end() {
        let out = evaluate(
            &WindowFunction::Lead { offset: 1 },
            &vals(&[10, 20, 30]),
            &vals(&[10, 20, 30]),
        )
        .unwrap();
        assert_eq!(out, vec![Value::Int64(20), Value::Int64(30), Value::Null]);
    }

    #[test]
    fn first_value_repeats_first_value() {
        let out = evaluate(
            &WindowFunction::FirstValue,
            &vals(&[10, 20, 30]),
            &vals(&[10, 20, 30]),
        )
        .unwrap();
        assert_eq!(
            out,
            vec![Value::Int64(10), Value::Int64(10), Value::Int64(10)]
        );
    }

    #[test]
    fn last_value_repeats_last_value() {
        let out = evaluate(
            &WindowFunction::LastValue,
            &vals(&[10, 20, 30]),
            &vals(&[10, 20, 30]),
        )
        .unwrap();
        assert_eq!(
            out,
            vec![Value::Int64(30), Value::Int64(30), Value::Int64(30)]
        );
    }

    #[test]
    fn ntile_distributes_extra_to_earlier_buckets() {
        // 7 rows / 3 buckets → bucket sizes 3,2,2.
        let out = evaluate(
            &WindowFunction::Ntile { buckets: 3 },
            &vals(&[1, 2, 3, 4, 5, 6, 7]),
            &vals(&[1, 2, 3, 4, 5, 6, 7]),
        )
        .unwrap();
        assert_eq!(
            out,
            vec![
                Value::Int64(1),
                Value::Int64(1),
                Value::Int64(1),
                Value::Int64(2),
                Value::Int64(2),
                Value::Int64(3),
                Value::Int64(3),
            ]
        );
    }

    #[test]
    fn ntile_with_zero_buckets_errors() {
        let err = evaluate(
            &WindowFunction::Ntile { buckets: 0 },
            &vals(&[1, 2]),
            &vals(&[1, 2]),
        )
        .unwrap_err();
        assert!(err.contains("> 0"));
    }

    #[test]
    fn window_function_names_are_lowercase() {
        assert_eq!(WindowFunction::RowNumber.name(), "row_number");
        assert_eq!(WindowFunction::Rank.name(), "rank");
        assert_eq!(WindowFunction::DenseRank.name(), "dense_rank");
        assert_eq!(WindowFunction::Lag { offset: 1 }.name(), "lag");
        assert_eq!(WindowFunction::Lead { offset: 1 }.name(), "lead");
        assert_eq!(WindowFunction::FirstValue.name(), "first_value");
        assert_eq!(WindowFunction::LastValue.name(), "last_value");
        assert_eq!(WindowFunction::Ntile { buckets: 4 }.name(), "ntile");
    }

    #[test]
    fn frame_struct_is_half_open_range() {
        let f = Frame { start: 0, end: 3 };
        assert_eq!(f.end - f.start, 3);
    }
}
