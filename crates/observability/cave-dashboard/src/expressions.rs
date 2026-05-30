// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Server-side expression engine — line-port of grafana/grafana `pkg/expr`
//! (`mathexp` reducers/resample + `classic` conditions + math operators).
//!
//! This is the "mixed-datasource expressions" surface: expression nodes
//! reference the results of upstream datasource queries by `refId` and run
//! reduce / resample / math / classic-condition operations entirely
//! server-side, exactly like Grafana's `__expr__` datasource.

// ── mathexp reducers — pkg/expr/mathexp/reduce.go ───────────────────────────

/// The reduction function applied to a series to produce a single number.
/// Mirrors `mathexp.ReducerID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReducerId {
    Sum,
    Mean,
    Min,
    Max,
    Count,
    Last,
    Median,
}

impl ReducerId {
    /// Parse a reducer name (case-insensitive), matching
    /// `mathexp.ReducerID(strings.ToLower(redString))`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "sum" => Some(ReducerId::Sum),
            "mean" => Some(ReducerId::Mean),
            "min" => Some(ReducerId::Min),
            "max" => Some(ReducerId::Max),
            "count" => Some(ReducerId::Count),
            "last" => Some(ReducerId::Last),
            "median" => Some(ReducerId::Median),
            _ => None,
        }
    }

    /// Supported reducer ids — `GetSupportedReduceFuncs()`.
    pub fn supported() -> &'static [ReducerId] {
        &[
            ReducerId::Sum,
            ReducerId::Mean,
            ReducerId::Min,
            ReducerId::Max,
            ReducerId::Count,
            ReducerId::Last,
            ReducerId::Median,
        ]
    }
}

#[inline]
fn is_nil_or_nan(v: Option<f64>) -> bool {
    match v {
        None => true,
        Some(f) => f.is_nan(),
    }
}

/// Sum — any nil/NaN element forces the whole reduction to NaN (Go `Sum`).
fn r_sum(vals: &[Option<f64>]) -> f64 {
    let mut sum = 0.0;
    for &v in vals {
        match v {
            Some(f) if !f.is_nan() => sum += f,
            _ => return f64::NAN,
        }
    }
    sum
}

/// Mean = Sum / Len (Go `Avg`). Len counts every point.
fn r_avg(vals: &[Option<f64>]) -> f64 {
    let sum = r_sum(vals);
    sum / vals.len() as f64
}

fn r_min(vals: &[Option<f64>]) -> f64 {
    if vals.is_empty() {
        return f64::NAN;
    }
    let mut f = 0.0;
    for (i, &v) in vals.iter().enumerate() {
        match v {
            Some(x) if !x.is_nan() => {
                if i == 0 || x < f {
                    f = x;
                }
            }
            _ => return f64::NAN,
        }
    }
    f
}

fn r_max(vals: &[Option<f64>]) -> f64 {
    if vals.is_empty() {
        return f64::NAN;
    }
    let mut f = 0.0;
    for (i, &v) in vals.iter().enumerate() {
        match v {
            Some(x) if !x.is_nan() => {
                if i == 0 || x > f {
                    f = x;
                }
            }
            _ => return f64::NAN,
        }
    }
    f
}

/// Count = number of points, including nils (Go `Count`).
fn r_count(vals: &[Option<f64>]) -> f64 {
    vals.len() as f64
}

fn r_last(vals: &[Option<f64>]) -> f64 {
    match vals.last() {
        None => f64::NAN,
        Some(None) => f64::NAN,
        Some(Some(f)) => *f,
    }
}

fn r_median(vals: &[Option<f64>]) -> f64 {
    let mut values: Vec<f64> = Vec::with_capacity(vals.len());
    for &v in vals {
        match v {
            Some(f) if !f.is_nan() => values.push(f),
            _ => return f64::NAN,
        }
    }
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// Reduce a series of optional values to a single `Number`, where `None`
/// represents a null result and `Some(NaN)` a NaN result — matching
/// Grafana's `Series.Reduce`. The reducer never returns a Go-nil here, so the
/// result is always `Some`.
pub fn reduce_series(reducer: ReducerId, vals: &[Option<f64>]) -> Option<f64> {
    let f = match reducer {
        ReducerId::Sum => r_sum(vals),
        ReducerId::Mean => r_avg(vals),
        ReducerId::Min => r_min(vals),
        ReducerId::Max => r_max(vals),
        ReducerId::Count => r_count(vals),
        ReducerId::Last => r_last(vals),
        ReducerId::Median => r_median(vals),
    };
    Some(f)
}

// ── ReduceMapper — pkg/expr/mathexp/reduce.go ───────────────────────────────

/// Maps points before/after reduction. Mirrors the `ReduceMapper` interface
/// implementations `DropNonNumber` and `ReplaceNonNumberWithValue`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ReduceMapper {
    /// Drop nil / NaN / Inf points entirely.
    DropNonNumber,
    /// Replace nil / NaN / Inf points with a fixed value.
    ReplaceNonNumber(f64),
}

impl ReduceMapper {
    /// `MapInput` — applied to each point of the source series.
    pub fn map_input(&self, v: Option<f64>) -> Option<f64> {
        match self {
            ReduceMapper::DropNonNumber => match v {
                Some(f) if !f.is_nan() && !f.is_infinite() => Some(f),
                _ => None,
            },
            ReduceMapper::ReplaceNonNumber(value) => match v {
                Some(f) if !f.is_nan() && !f.is_infinite() => Some(f),
                _ => Some(*value),
            },
        }
    }

    /// `MapOutput` — applied to the reduced result.
    pub fn map_output(&self, v: Option<f64>) -> Option<f64> {
        match self {
            ReduceMapper::DropNonNumber => match v {
                Some(f) if f.is_nan() => None,
                other => other,
            },
            ReduceMapper::ReplaceNonNumber(value) => match v {
                Some(f) if f.is_nan() => Some(*value),
                other => other,
            },
        }
    }
}

/// Reduce with a mapper — the source series is mapped (dropping `None`
/// results) before reduction, then the output is mapped, matching
/// `Series.Reduce` with a non-nil `mapper`.
pub fn reduce_series_mapped(
    reducer: ReducerId,
    vals: &[Option<f64>],
    mapper: &ReduceMapper,
) -> Option<f64> {
    let mapped: Vec<Option<f64>> = vals
        .iter()
        .filter_map(|&v| mapper.map_input(v).map(Some))
        .collect();
    let reduced = reduce_series(reducer, &mapped);
    mapper.map_output(reduced)
}

// ── resample — pkg/expr/mathexp/resample.go ─────────────────────────────────

/// Upsampling strategy when a resample bucket has no source points.
/// Mirrors `mathexp.Upsampler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Upsampler {
    /// Use the last seen value (`pad`).
    Pad,
    /// Backfill with the next available value (`backfilling`).
    Backfill,
    /// Do not fill — leave null (`fillna`).
    FillNa,
}

impl Upsampler {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pad" => Some(Upsampler::Pad),
            "backfilling" => Some(Upsampler::Backfill),
            "fillna" => Some(Upsampler::FillNa),
            _ => None,
        }
    }
}

/// Resample a `(times, vals)` series onto a uniform grid from `from` to `to`
/// at `interval`, downsampling multi-point buckets with `downsampler` and
/// filling empty buckets with `upsampler`. Time units are arbitrary integers
/// (e.g. seconds or nanoseconds), matching `Series.Resample`.
#[allow(clippy::too_many_arguments)]
pub fn resample(
    times: &[i64],
    vals: &[Option<f64>],
    from: i64,
    to: i64,
    interval: i64,
    downsampler: ReducerId,
    upsampler: Upsampler,
) -> Result<(Vec<i64>, Vec<Option<f64>>), String> {
    if interval <= 0 {
        return Err("resample interval must be positive".to_string());
    }
    let new_series_length = (to - from) / interval;
    if new_series_length <= 0 {
        return Err(
            "the series cannot be sampled further; the time range is shorter than the interval"
                .to_string(),
        );
    }
    let len = times.len().min(vals.len());
    let mut out_times = Vec::with_capacity((new_series_length + 1) as usize);
    let mut out_vals = Vec::with_capacity((new_series_length + 1) as usize);

    let mut bookmark = 0usize;
    let mut last_seen: Option<f64> = None;
    let mut idx: i64 = 0;
    let mut t = from;
    while t <= to && idx <= new_series_length {
        let mut bucket: Vec<Option<f64>> = Vec::new();
        let mut s_idx = bookmark;
        loop {
            if s_idx == len {
                break;
            }
            let st = times[s_idx];
            if st > t {
                break;
            }
            bookmark += 1;
            s_idx += 1;
            last_seen = vals[s_idx - 1];
            bucket.push(vals[s_idx - 1]);
        }

        let value: Option<f64> = if bucket.is_empty() {
            // upsampling
            match upsampler {
                Upsampler::Pad => last_seen,
                Upsampler::Backfill => {
                    if s_idx == len {
                        None
                    } else {
                        vals[s_idx]
                    }
                }
                Upsampler::FillNa => None,
            }
        } else if bucket.len() == 1 {
            bucket[0]
        } else {
            // downsampling
            let v = match downsampler {
                ReducerId::Sum => r_sum(&bucket),
                ReducerId::Mean => r_avg(&bucket),
                ReducerId::Min => r_min(&bucket),
                ReducerId::Max => r_max(&bucket),
                ReducerId::Last => r_last(&bucket),
                other => {
                    return Err(format!("downsampling {other:?} not implemented"));
                }
            };
            Some(v)
        };

        out_times.push(t);
        out_vals.push(value);
        t += interval;
        idx += 1;
    }
    Ok((out_times, out_vals))
}

// ── classic conditions — pkg/expr/classic/{reduce,evaluator,classic}.go ─────

/// Reducer used inside a classic condition. Unlike the `mathexp` reducers,
/// these skip nulls and return a null result for all-null series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassicReducer {
    Avg,
    Sum,
    Min,
    Max,
    Count,
    Last,
    Median,
    Diff,
    DiffAbs,
    PercentDiff,
    PercentDiffAbs,
    CountNonNull,
}

impl ClassicReducer {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "avg" => Some(ClassicReducer::Avg),
            "sum" => Some(ClassicReducer::Sum),
            "min" => Some(ClassicReducer::Min),
            "max" => Some(ClassicReducer::Max),
            "count" => Some(ClassicReducer::Count),
            "last" => Some(ClassicReducer::Last),
            "median" => Some(ClassicReducer::Median),
            "diff" => Some(ClassicReducer::Diff),
            "diff_abs" => Some(ClassicReducer::DiffAbs),
            "percent_diff" => Some(ClassicReducer::PercentDiff),
            "percent_diff_abs" => Some(ClassicReducer::PercentDiffAbs),
            "count_non_null" => Some(ClassicReducer::CountNonNull),
            _ => None,
        }
    }
}

fn classic_diff(vals: &[Option<f64>], mut all_null: bool, mut value: f64, f: fn(f64, f64) -> f64) -> (bool, f64) {
    let mut first = 0.0;
    // newest non-null point, scanning from the end
    let mut newest_idx: isize = -1;
    for i in (0..vals.len()).rev() {
        if !is_nil_or_nan(vals[i]) {
            first = vals[i].unwrap();
            all_null = false;
            newest_idx = i as isize;
            break;
        }
    }
    if newest_idx >= 1 {
        // oldest non-null point, scanning from the start
        for &v in vals.iter() {
            if !is_nil_or_nan(v) {
                value = f(first, v.unwrap());
                all_null = false;
                break;
            }
        }
    }
    (all_null, value)
}

/// Reduce a series to a single number under classic-condition semantics.
/// Returns `None` when the result is null (all-null series), matching
/// `reducer.Reduce` which leaves the `Number` value unset.
pub fn classic_reduce(reducer: ClassicReducer, vals: &[Option<f64>]) -> Option<f64> {
    if vals.is_empty() {
        return None;
    }
    let mut value = 0.0f64;
    let mut all_null = true;

    match reducer {
        ClassicReducer::Avg => {
            let mut valid = 0;
            for &v in vals {
                if is_nil_or_nan(v) {
                    continue;
                }
                value += v.unwrap();
                valid += 1;
                all_null = false;
            }
            if valid > 0 {
                value /= valid as f64;
            }
        }
        ClassicReducer::Sum => {
            for &v in vals {
                if is_nil_or_nan(v) {
                    continue;
                }
                value += v.unwrap();
                all_null = false;
            }
        }
        ClassicReducer::Min => {
            value = f64::MAX;
            for &v in vals {
                if is_nil_or_nan(v) {
                    continue;
                }
                all_null = false;
                if value > v.unwrap() {
                    value = v.unwrap();
                }
            }
            if all_null {
                value = 0.0;
            }
        }
        ClassicReducer::Max => {
            value = -f64::MAX;
            for &v in vals {
                if is_nil_or_nan(v) {
                    continue;
                }
                all_null = false;
                if value < v.unwrap() {
                    value = v.unwrap();
                }
            }
            if all_null {
                value = 0.0;
            }
        }
        ClassicReducer::Count => {
            value = vals.len() as f64;
            all_null = false;
        }
        ClassicReducer::Last => {
            for &v in vals.iter().rev() {
                if !is_nil_or_nan(v) {
                    value = v.unwrap();
                    all_null = false;
                    break;
                }
            }
        }
        ClassicReducer::Median => {
            let mut values: Vec<f64> = Vec::new();
            for &v in vals {
                if is_nil_or_nan(v) {
                    continue;
                }
                all_null = false;
                values.push(v.unwrap());
            }
            if !values.is_empty() {
                values.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let length = values.len();
                if length % 2 == 1 {
                    value = values[(length - 1) / 2];
                } else {
                    value = (values[(length / 2) - 1] + values[length / 2]) / 2.0;
                }
            }
        }
        ClassicReducer::Diff => {
            (all_null, value) = classic_diff(vals, all_null, value, |n, o| n - o);
        }
        ClassicReducer::DiffAbs => {
            (all_null, value) = classic_diff(vals, all_null, value, |n, o| (n - o).abs());
        }
        ClassicReducer::PercentDiff => {
            (all_null, value) = classic_diff(vals, all_null, value, |n, o| (n - o) / o.abs() * 100.0);
        }
        ClassicReducer::PercentDiffAbs => {
            (all_null, value) = classic_diff(vals, all_null, value, |n, o| ((n - o) / o * 100.0).abs());
        }
        ClassicReducer::CountNonNull => {
            for &v in vals {
                if is_nil_or_nan(v) {
                    continue;
                }
                value += 1.0;
            }
            if value > 0.0 {
                all_null = false;
            }
        }
    }

    if all_null {
        return None;
    }
    Some(value)
}

/// Classic-condition evaluator. Mirrors `classic.evaluator` implementations.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Evaluator {
    Threshold { typ: String, threshold: f64 },
    Ranged { typ: String, lower: f64, upper: f64 },
    NoValue,
}

/// Evaluate a reduced value against a condition evaluator.
/// A null (`None`) reduced value is only firing for the `no_value` evaluator.
pub fn eval_condition(e: &Evaluator, reduced: Option<f64>) -> bool {
    match e {
        Evaluator::NoValue => reduced.is_none(),
        Evaluator::Threshold { typ, threshold } => {
            let Some(fv) = reduced else { return false };
            match typ.as_str() {
                "gt" => fv > *threshold,
                "lt" => fv < *threshold,
                _ => false,
            }
        }
        Evaluator::Ranged { typ, lower, upper } => {
            let Some(fv) = reduced else { return false };
            match typ.as_str() {
                "within_range" => (*lower < fv && *upper > fv) || (*upper < fv && *lower > fv),
                "outside_range" => (*upper < fv && *lower < fv) || (*upper > fv && *lower > fv),
                _ => false,
            }
        }
    }
}

/// Logical operator joining successive conditions in a classic command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConditionOperator {
    And,
    Or,
    LogicOr,
}

/// `compareWithOperator` — or / logic-or use OR, everything else uses AND.
pub fn combine_conditions(b1: bool, b2: bool, op: ConditionOperator) -> bool {
    match op {
        ConditionOperator::Or | ConditionOperator::LogicOr => b1 || b2,
        ConditionOperator::And => b1 && b2,
    }
}

// ── math binary operators — pkg/expr/mathexp/exp.go binaryOp ────────────────

/// Apply a binary math operator to two scalars, faithfully reproducing
/// Grafana's `binaryOp`: short-circuit `||`/`&&`, NaN propagation, and
/// comparison/logical ops returning 1.0 or 0.0.
pub fn math_binop_scalar(op: &str, a: f64, b: f64) -> Result<f64, String> {
    // Short circuit before the NaN check.
    match op {
        "||" if a != 0.0 => return Ok(1.0),
        "&&" if a == 0.0 => return Ok(0.0),
        _ => {}
    }
    if a.is_nan() || b.is_nan() {
        return Ok(f64::NAN);
    }
    let r = match op {
        "+" => a + b,
        "*" => a * b,
        "-" => a - b,
        "/" => a / b,
        "**" => a.powf(b),
        "%" => a % b,
        "==" => (a == b) as i32 as f64,
        ">" => (a > b) as i32 as f64,
        "!=" => (a != b) as i32 as f64,
        "<" => (a < b) as i32 as f64,
        ">=" => (a >= b) as i32 as f64,
        "<=" => (a <= b) as i32 as f64,
        "||" => (a != 0.0 || b != 0.0) as i32 as f64,
        "&&" => (a != 0.0 && b != 0.0) as i32 as f64,
        _ => return Err(format!("expr: unknown operator {op}")),
    };
    Ok(r)
}

/// Number-vs-Number binary op: a null (`None`) operand yields a null result
/// (`biScalarNumber` semantics), otherwise applies [`math_binop_scalar`].
pub fn math_binop(op: &str, a: Option<f64>, b: Option<f64>) -> Result<Option<f64>, String> {
    match (a, b) {
        (Some(x), Some(y)) => Ok(Some(math_binop_scalar(op, x, y)?)),
        _ => {
            // Validate the operator even when short-circuiting to null.
            math_binop_scalar(op, 0.0, 0.0)?;
            Ok(None)
        }
    }
}

// ── DataFrame integration ───────────────────────────────────────────────────

/// Extract the `time` and `number` columns from a Grafana-shaped time-series
/// DataFrame into `(times, values)`, with JSON `null` mapped to `None`.
pub fn extract_number_series(
    frame: &crate::models::DataFrame,
) -> (Vec<i64>, Vec<Option<f64>>) {
    let time_idx = frame
        .schema
        .fields
        .iter()
        .position(|f| f.field_type == "time");
    let num_idx = frame
        .schema
        .fields
        .iter()
        .position(|f| f.field_type == "number");

    let times = match time_idx.and_then(|i| frame.data.values.get(i)) {
        Some(col) => col.iter().map(json_to_i64).collect(),
        None => Vec::new(),
    };
    let vals = match num_idx.and_then(|i| frame.data.values.get(i)) {
        Some(col) => col.iter().map(json_to_f64).collect(),
        None => Vec::new(),
    };
    (times, vals)
}

fn json_to_i64(v: &serde_json::Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .unwrap_or(0)
}

fn json_to_f64(v: &serde_json::Value) -> Option<f64> {
    if v.is_null() {
        None
    } else {
        v.as_f64()
    }
}

/// Build a single-value "Number" DataFrame for a refId, with `None` rendered
/// as a JSON `null`.
pub fn number_frame(ref_id: &str, value: Option<f64>) -> crate::models::DataFrame {
    use crate::models::{DataFrame, DataFrameData, DataFrameSchema, FieldSchema};
    let cell = match value {
        Some(f) => serde_json::json!(f),
        None => serde_json::Value::Null,
    };
    DataFrame {
        schema: DataFrameSchema {
            ref_id: ref_id.to_string(),
            name: ref_id.to_string(),
            fields: vec![FieldSchema {
                name: ref_id.to_string(),
                field_type: "number".to_string(),
                type_info: None,
                labels: None,
                config: None,
            }],
            meta: None,
        },
        data: DataFrameData {
            values: vec![vec![cell]],
            entities: None,
        },
    }
}

/// Read the single value from a Number frame (the first number field, row 0).
pub fn value_of_number_frame(frame: &crate::models::DataFrame) -> Option<f64> {
    let num_idx = frame
        .schema
        .fields
        .iter()
        .position(|f| f.field_type == "number")?;
    let col = frame.data.values.get(num_idx)?;
    json_to_f64(col.first()?)
}

// ── high-level cross-refId expression pipeline ──────────────────────────────

/// A runtime value flowing through the expression graph — either a time
/// series or a reduced single number (`None` = null).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "lowercase")]
pub enum ExprValue {
    Series {
        times: Vec<i64>,
        vals: Vec<Option<f64>>,
    },
    Number(Option<f64>),
}

impl ExprValue {
    /// Coerce to a single number: a Series is reduced via [`reduce_series`]
    /// (used where the model expects a scalar but received a series).
    fn as_number(&self, reducer: ReducerId) -> Option<f64> {
        match self {
            ExprValue::Number(v) => *v,
            ExprValue::Series { vals, .. } => reduce_series(reducer, vals),
        }
    }
}

/// A math operand — a reference to another node's result, or a literal.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operand {
    Ref(String),
    Lit(f64),
}

/// A single condition within a classic-condition command.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClassicCond {
    pub input: String,
    pub reducer: ClassicReducer,
    pub evaluator: Evaluator,
    pub operator: ConditionOperator,
}

/// A server-side expression command referencing upstream results by refId.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExprCommand {
    Reduce {
        input: String,
        reducer: ReducerId,
        mapper: Option<ReduceMapper>,
    },
    Resample {
        input: String,
        from: i64,
        to: i64,
        interval: i64,
        downsampler: ReducerId,
        upsampler: Upsampler,
    },
    MathBinary {
        left: Operand,
        op: String,
        right: Operand,
    },
    Threshold {
        input: String,
        evaluator: Evaluator,
    },
    ClassicCondition {
        conditions: Vec<ClassicCond>,
    },
}

fn lookup<'a>(
    vars: &'a std::collections::HashMap<String, ExprValue>,
    ref_id: &str,
) -> Result<&'a ExprValue, String> {
    vars.get(ref_id)
        .ok_or_else(|| format!("expression references unknown refId '{ref_id}'"))
}

/// Evaluate one expression command against the map of upstream results.
pub fn evaluate(
    cmd: &ExprCommand,
    vars: &std::collections::HashMap<String, ExprValue>,
) -> Result<ExprValue, String> {
    match cmd {
        ExprCommand::Reduce {
            input,
            reducer,
            mapper,
        } => {
            let v = lookup(vars, input)?;
            let vals = match v {
                ExprValue::Series { vals, .. } => vals.as_slice(),
                ExprValue::Number(n) => {
                    // Reducing a number returns the number unchanged.
                    return Ok(ExprValue::Number(*n));
                }
            };
            let result = match mapper {
                Some(m) => reduce_series_mapped(*reducer, vals, m),
                None => reduce_series(*reducer, vals),
            };
            Ok(ExprValue::Number(result))
        }
        ExprCommand::Resample {
            input,
            from,
            to,
            interval,
            downsampler,
            upsampler,
        } => {
            let v = lookup(vars, input)?;
            match v {
                ExprValue::Series { times, vals } => {
                    let (t, vv) =
                        resample(times, vals, *from, *to, *interval, *downsampler, *upsampler)?;
                    Ok(ExprValue::Series { times: t, vals: vv })
                }
                ExprValue::Number(_) => {
                    Err("resample input must be a series".to_string())
                }
            }
        }
        ExprCommand::MathBinary { left, op, right } => {
            let resolve = |o: &Operand| -> Result<Option<f64>, String> {
                match o {
                    Operand::Lit(f) => Ok(Some(*f)),
                    Operand::Ref(r) => Ok(lookup(vars, r)?.as_number(ReducerId::Last)),
                }
            };
            let a = resolve(left)?;
            let b = resolve(right)?;
            Ok(ExprValue::Number(math_binop(op, a, b)?))
        }
        ExprCommand::Threshold { input, evaluator } => {
            let v = lookup(vars, input)?;
            let num = v.as_number(ReducerId::Last);
            // A null input is no-data (nil); otherwise fire → 1.0 else 0.0.
            if num.is_none() {
                return Ok(ExprValue::Number(None));
            }
            let firing = eval_condition(evaluator, num);
            Ok(ExprValue::Number(Some(if firing { 1.0 } else { 0.0 })))
        }
        ExprCommand::ClassicCondition { conditions } => {
            let mut is_firing = false;
            let mut is_nodata = false;
            for (i, cond) in conditions.iter().enumerate() {
                if is_firing && cond.operator == ConditionOperator::LogicOr {
                    break;
                }
                let v = lookup(vars, &cond.input)?;
                let number = match v {
                    ExprValue::Number(n) => *n,
                    ExprValue::Series { vals, .. } => classic_reduce(cond.reducer, vals),
                };
                let cond_firing = eval_condition(&cond.evaluator, number);
                let cond_nodata = number.is_none();
                if i == 0 {
                    is_firing = cond_firing;
                    is_nodata = cond_nodata;
                } else {
                    is_firing = combine_conditions(is_firing, cond_firing, cond.operator);
                    is_nodata = combine_conditions(is_nodata, cond_nodata, cond.operator);
                }
            }
            // isNoData is checked first (both can be true simultaneously).
            let value = if is_nodata {
                None
            } else if is_firing {
                Some(1.0)
            } else {
                Some(0.0)
            };
            Ok(ExprValue::Number(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DataFrame, DataFrameData, DataFrameSchema, FieldSchema};
    use std::collections::HashMap;

    /// Build a Grafana-shaped time-series DataFrame: a `time` field + a
    /// `number` field with the given values.
    fn test_series_frame(ref_id: &str, times: &[i64], vals: &[Option<f64>]) -> DataFrame {
        let time_col: Vec<serde_json::Value> =
            times.iter().map(|t| serde_json::json!(t)).collect();
        let val_col: Vec<serde_json::Value> = vals
            .iter()
            .map(|v| match v {
                Some(f) => serde_json::json!(f),
                None => serde_json::Value::Null,
            })
            .collect();
        DataFrame {
            schema: DataFrameSchema {
                ref_id: ref_id.to_string(),
                name: ref_id.to_string(),
                fields: vec![
                    FieldSchema { name: "time".into(), field_type: "time".into(), type_info: None, labels: None, config: None },
                    FieldSchema { name: "value".into(), field_type: "number".into(), type_info: None, labels: None, config: None },
                ],
                meta: None,
            },
            data: DataFrameData { values: vec![time_col, val_col], entities: None },
        }
    }

    // ── mathexp reducers (pkg/expr/mathexp/reduce.go) ───────────────────────

    #[test]
    fn test_reducer_id_parse_case_insensitive() {
        assert_eq!(ReducerId::parse("sum"), Some(ReducerId::Sum));
        assert_eq!(ReducerId::parse("MEAN"), Some(ReducerId::Mean));
        assert_eq!(ReducerId::parse("Min"), Some(ReducerId::Min));
        assert_eq!(ReducerId::parse("max"), Some(ReducerId::Max));
        assert_eq!(ReducerId::parse("count"), Some(ReducerId::Count));
        assert_eq!(ReducerId::parse("last"), Some(ReducerId::Last));
        assert_eq!(ReducerId::parse("median"), Some(ReducerId::Median));
        assert_eq!(ReducerId::parse("bogus"), None);
    }

    #[test]
    fn test_reduce_sum() {
        assert_eq!(reduce_series(ReducerId::Sum, &[Some(1.0), Some(2.0), Some(3.0)]), Some(6.0));
    }

    #[test]
    fn test_reduce_sum_with_null_propagates_nan() {
        // Go Sum: any nil/NaN element → returns NaN for the whole reduction.
        let r = reduce_series(ReducerId::Sum, &[Some(1.0), None, Some(3.0)]).unwrap();
        assert!(r.is_nan());
    }

    #[test]
    fn test_reduce_mean_divides_by_len() {
        // Avg = Sum / Len  (Len counts every point, incl. would-be nils).
        assert_eq!(reduce_series(ReducerId::Mean, &[Some(2.0), Some(4.0)]), Some(3.0));
        assert_eq!(reduce_series(ReducerId::Mean, &[Some(1.0), Some(2.0), Some(3.0)]), Some(2.0));
    }

    #[test]
    fn test_reduce_min_max() {
        assert_eq!(reduce_series(ReducerId::Min, &[Some(3.0), Some(1.0), Some(2.0)]), Some(1.0));
        assert_eq!(reduce_series(ReducerId::Max, &[Some(3.0), Some(1.0), Some(2.0)]), Some(3.0));
    }

    #[test]
    fn test_reduce_min_with_null_is_nan() {
        assert!(reduce_series(ReducerId::Min, &[Some(3.0), None]).unwrap().is_nan());
    }

    #[test]
    fn test_reduce_min_empty_is_nan() {
        assert!(reduce_series(ReducerId::Min, &[]).unwrap().is_nan());
    }

    #[test]
    fn test_reduce_count_is_length_including_nulls() {
        // Go Count = float64(fv.Len()) — nulls still counted.
        assert_eq!(reduce_series(ReducerId::Count, &[Some(1.0), None, Some(3.0)]), Some(3.0));
    }

    #[test]
    fn test_reduce_last() {
        assert_eq!(reduce_series(ReducerId::Last, &[Some(1.0), Some(2.0), Some(3.0)]), Some(3.0));
    }

    #[test]
    fn test_reduce_last_empty_is_nan() {
        assert!(reduce_series(ReducerId::Last, &[]).unwrap().is_nan());
    }

    #[test]
    fn test_reduce_median_odd_and_even() {
        assert_eq!(reduce_series(ReducerId::Median, &[Some(3.0), Some(1.0), Some(2.0)]), Some(2.0));
        assert_eq!(
            reduce_series(ReducerId::Median, &[Some(1.0), Some(2.0), Some(3.0), Some(4.0)]),
            Some(2.5)
        );
    }

    #[test]
    fn test_reduce_median_with_null_is_nan() {
        assert!(reduce_series(ReducerId::Median, &[Some(1.0), None]).unwrap().is_nan());
    }

    // ── ReduceMapper (DropNonNumber / ReplaceNonNumberWithValue) ────────────

    #[test]
    fn test_drop_non_number_map_input() {
        let m = ReduceMapper::DropNonNumber;
        assert_eq!(m.map_input(Some(5.0)), Some(5.0));
        assert_eq!(m.map_input(None), None);
        assert_eq!(m.map_input(Some(f64::NAN)), None);
        assert_eq!(m.map_input(Some(f64::INFINITY)), None);
        assert_eq!(m.map_input(Some(f64::NEG_INFINITY)), None);
    }

    #[test]
    fn test_replace_non_number_map_input() {
        let m = ReduceMapper::ReplaceNonNumber(0.0);
        assert_eq!(m.map_input(Some(5.0)), Some(5.0));
        assert_eq!(m.map_input(None), Some(0.0));
        assert_eq!(m.map_input(Some(f64::NAN)), Some(0.0));
        assert_eq!(m.map_input(Some(f64::INFINITY)), Some(0.0));
    }

    #[test]
    fn test_reduce_with_drop_mapper_drops_then_sums() {
        // DropNonNumber maps the input series first: [1, NaN, 3] -> [1, 3] -> Sum = 4.
        let r = reduce_series_mapped(
            ReducerId::Sum,
            &[Some(1.0), Some(f64::NAN), Some(3.0)],
            &ReduceMapper::DropNonNumber,
        );
        assert_eq!(r, Some(4.0));
    }

    #[test]
    fn test_reduce_with_replace_mapper() {
        // ReplaceNonNumber(0): [1, NaN, 3] -> [1, 0, 3] -> Sum = 4.
        let r = reduce_series_mapped(
            ReducerId::Sum,
            &[Some(1.0), Some(f64::NAN), Some(3.0)],
            &ReduceMapper::ReplaceNonNumber(0.0),
        );
        assert_eq!(r, Some(4.0));
    }

    // ── resample (pkg/expr/mathexp/resample.go) ─────────────────────────────

    #[test]
    fn test_upsampler_parse() {
        assert_eq!(Upsampler::parse("pad"), Some(Upsampler::Pad));
        assert_eq!(Upsampler::parse("backfilling"), Some(Upsampler::Backfill));
        assert_eq!(Upsampler::parse("fillna"), Some(Upsampler::FillNa));
        assert_eq!(Upsampler::parse("nope"), None);
    }

    #[test]
    fn test_resample_range_shorter_than_interval_errors() {
        // from=0 to=4 interval=5 -> newSeriesLength = 0 -> error.
        let r = resample(&[2, 7], &[Some(2.0), Some(1.0)], 0, 4, 5, ReducerId::Mean, Upsampler::FillNa);
        assert!(r.is_err());
    }

    #[test]
    fn test_resample_invalid_time_range_errors() {
        let r = resample(&[2, 7], &[Some(2.0), Some(1.0)], 11, 0, 5, ReducerId::Mean, Upsampler::FillNa);
        assert!(r.is_err());
    }

    #[test]
    fn test_resample_downsample_mean_pad() {
        // source (2,2)(4,3)(7,1)(9,2); from=0 to=16 interval=5; mean / pad.
        let (times, vals) = resample(
            &[2, 4, 7, 9],
            &[Some(2.0), Some(3.0), Some(1.0), Some(2.0)],
            0,
            16,
            5,
            ReducerId::Mean,
            Upsampler::Pad,
        )
        .unwrap();
        assert_eq!(times, vec![0, 5, 10, 15]);
        assert_eq!(vals, vec![None, Some(2.5), Some(1.5), Some(2.0)]);
    }

    #[test]
    fn test_resample_downsample_max_fillna() {
        let (times, vals) = resample(
            &[2, 4, 7, 9],
            &[Some(2.0), Some(3.0), Some(1.0), Some(2.0)],
            0,
            16,
            5,
            ReducerId::Max,
            Upsampler::FillNa,
        )
        .unwrap();
        assert_eq!(times, vec![0, 5, 10, 15]);
        assert_eq!(vals, vec![None, Some(3.0), Some(2.0), None]);
    }

    #[test]
    fn test_resample_upsample_backfill() {
        // source (7,1)(9,2)(12,5); from=0 to=16 interval=5; mean / backfilling.
        let (times, vals) = resample(
            &[7, 9, 12],
            &[Some(1.0), Some(2.0), Some(5.0)],
            0,
            16,
            5,
            ReducerId::Mean,
            Upsampler::Backfill,
        )
        .unwrap();
        assert_eq!(times, vec![0, 5, 10, 15]);
        assert_eq!(vals, vec![Some(1.0), Some(1.0), Some(1.5), Some(5.0)]);
    }

    #[test]
    fn test_resample_downsample_count_unsupported() {
        // Go resample only supports sum/mean/min/max/last downsamplers.
        let r = resample(
            &[2, 4],
            &[Some(2.0), Some(3.0)],
            0,
            16,
            5,
            ReducerId::Count,
            Upsampler::Pad,
        );
        assert!(r.is_err());
    }

    // ── classic condition reducer (pkg/expr/classic/reduce.go) ──────────────

    #[test]
    fn test_classic_reducer_valid() {
        for r in ["avg", "sum", "min", "max", "count", "last", "median", "diff", "diff_abs",
                  "percent_diff", "percent_diff_abs", "count_non_null"] {
            assert!(ClassicReducer::parse(r).is_some(), "{r} should be valid");
        }
        assert!(ClassicReducer::parse("bogus").is_none());
    }

    #[test]
    fn test_classic_reduce_avg_skips_nulls() {
        // avg divides by the count of valid points only.
        assert_eq!(classic_reduce(ClassicReducer::Avg, &[Some(1.0), None, Some(3.0)]), Some(2.0));
    }

    #[test]
    fn test_classic_reduce_sum_min_max_skip_nulls() {
        assert_eq!(classic_reduce(ClassicReducer::Sum, &[Some(1.0), None, Some(3.0)]), Some(4.0));
        assert_eq!(classic_reduce(ClassicReducer::Min, &[Some(3.0), None, Some(1.0)]), Some(1.0));
        assert_eq!(classic_reduce(ClassicReducer::Max, &[Some(3.0), None, Some(1.0)]), Some(3.0));
    }

    #[test]
    fn test_classic_reduce_all_null_is_none() {
        // min/max over an all-null series produce a null result.
        assert_eq!(classic_reduce(ClassicReducer::Min, &[None, None]), None);
        assert_eq!(classic_reduce(ClassicReducer::Avg, &[None, None]), None);
    }

    #[test]
    fn test_classic_reduce_empty_is_none() {
        assert_eq!(classic_reduce(ClassicReducer::Sum, &[]), None);
    }

    #[test]
    fn test_classic_reduce_count_counts_all_points() {
        assert_eq!(classic_reduce(ClassicReducer::Count, &[Some(1.0), None, Some(3.0)]), Some(3.0));
    }

    #[test]
    fn test_classic_reduce_count_non_null() {
        assert_eq!(classic_reduce(ClassicReducer::CountNonNull, &[Some(1.0), None, Some(3.0)]), Some(2.0));
        assert_eq!(classic_reduce(ClassicReducer::CountNonNull, &[None, None]), None);
    }

    #[test]
    fn test_classic_reduce_last_non_null() {
        assert_eq!(classic_reduce(ClassicReducer::Last, &[Some(1.0), Some(2.0), None]), Some(2.0));
    }

    #[test]
    fn test_classic_reduce_diff_and_percent() {
        // diff = newest - oldest (non-null); newest scanned from the end.
        assert_eq!(classic_reduce(ClassicReducer::Diff, &[Some(1.0), None, Some(5.0)]), Some(4.0));
        assert_eq!(classic_reduce(ClassicReducer::DiffAbs, &[Some(5.0), Some(1.0)]), Some(4.0));
        // percent_diff = (newest - oldest) / |oldest| * 100
        assert_eq!(classic_reduce(ClassicReducer::PercentDiff, &[Some(1.0), Some(5.0)]), Some(400.0));
    }

    // ── classic evaluator (pkg/expr/classic/evaluator.go) ───────────────────

    #[test]
    fn test_eval_threshold_gt_lt() {
        let gt = Evaluator::Threshold { typ: "gt".into(), threshold: 10.0 };
        assert!(eval_condition(&gt, Some(11.0)));
        assert!(!eval_condition(&gt, Some(10.0)));
        let lt = Evaluator::Threshold { typ: "lt".into(), threshold: 10.0 };
        assert!(eval_condition(&lt, Some(9.0)));
        assert!(!eval_condition(&lt, Some(10.0)));
    }

    #[test]
    fn test_eval_threshold_null_is_false() {
        let gt = Evaluator::Threshold { typ: "gt".into(), threshold: 10.0 };
        assert!(!eval_condition(&gt, None));
    }

    #[test]
    fn test_eval_within_and_outside_range() {
        let within = Evaluator::Ranged { typ: "within_range".into(), lower: 10.0, upper: 100.0 };
        assert!(eval_condition(&within, Some(50.0)));
        assert!(!eval_condition(&within, Some(5.0)));
        let outside = Evaluator::Ranged { typ: "outside_range".into(), lower: 10.0, upper: 100.0 };
        assert!(eval_condition(&outside, Some(5.0)));
        assert!(!eval_condition(&outside, Some(50.0)));
    }

    #[test]
    fn test_eval_no_value() {
        let nv = Evaluator::NoValue;
        assert!(eval_condition(&nv, None));
        assert!(!eval_condition(&nv, Some(1.0)));
    }

    #[test]
    fn test_condition_operator_combine() {
        assert!(!combine_conditions(true, false, ConditionOperator::And));
        assert!(combine_conditions(true, false, ConditionOperator::Or));
        assert!(combine_conditions(false, true, ConditionOperator::LogicOr));
        assert!(combine_conditions(true, true, ConditionOperator::And));
    }

    // ── math binary operators (pkg/expr/mathexp/exp.go binaryOp) ────────────

    #[test]
    fn test_math_binop_arithmetic() {
        assert_eq!(math_binop_scalar("+", 2.0, 3.0).unwrap(), 5.0);
        assert_eq!(math_binop_scalar("-", 5.0, 3.0).unwrap(), 2.0);
        assert_eq!(math_binop_scalar("*", 2.0, 3.0).unwrap(), 6.0);
        assert_eq!(math_binop_scalar("/", 6.0, 2.0).unwrap(), 3.0);
        assert_eq!(math_binop_scalar("%", 7.0, 3.0).unwrap(), 1.0);
        assert_eq!(math_binop_scalar("**", 2.0, 3.0).unwrap(), 8.0);
    }

    #[test]
    fn test_math_binop_comparisons_return_one_or_zero() {
        assert_eq!(math_binop_scalar("==", 2.0, 2.0).unwrap(), 1.0);
        assert_eq!(math_binop_scalar("==", 2.0, 3.0).unwrap(), 0.0);
        assert_eq!(math_binop_scalar(">", 3.0, 2.0).unwrap(), 1.0);
        assert_eq!(math_binop_scalar("<=", 2.0, 2.0).unwrap(), 1.0);
        assert_eq!(math_binop_scalar("!=", 2.0, 3.0).unwrap(), 1.0);
    }

    #[test]
    fn test_math_binop_logical_short_circuit() {
        // && short circuits to 0 when a == 0, even if b is NaN.
        assert_eq!(math_binop_scalar("&&", 0.0, f64::NAN).unwrap(), 0.0);
        // || short circuits to 1 when a != 0, even if b is NaN.
        assert_eq!(math_binop_scalar("||", 1.0, f64::NAN).unwrap(), 1.0);
    }

    #[test]
    fn test_math_binop_nan_propagates() {
        assert!(math_binop_scalar("+", f64::NAN, 3.0).unwrap().is_nan());
    }

    #[test]
    fn test_math_binop_unknown_op_errors() {
        assert!(math_binop_scalar("???", 1.0, 2.0).is_err());
    }

    #[test]
    fn test_math_binop_option_nil_propagates() {
        // Number <op> Number where either is nil → nil.
        assert_eq!(math_binop("+", None, Some(3.0)).unwrap(), None);
        assert_eq!(math_binop("+", Some(2.0), Some(3.0)).unwrap(), Some(5.0));
    }

    // ── DataFrame integration ───────────────────────────────────────────────

    #[test]
    fn test_extract_number_series() {
        let frame = test_series_frame("A", &[0, 5, 10], &[Some(1.0), None, Some(3.0)]);
        let (times, vals) = extract_number_series(&frame);
        assert_eq!(times, vec![0, 5, 10]);
        assert_eq!(vals, vec![Some(1.0), None, Some(3.0)]);
    }

    #[test]
    fn test_number_frame_roundtrip() {
        let f = number_frame("A", Some(3.0));
        // a Number frame has a single number field with one row
        match value_of_number_frame(&f) {
            Some(v) => assert_eq!(v, 3.0),
            None => panic!("expected a value"),
        }
        assert_eq!(value_of_number_frame(&number_frame("B", None)), None);
    }

    // ── high-level cross-refId expression pipeline ──────────────────────────

    #[test]
    fn test_evaluate_reduce_over_series() {
        let mut vars = HashMap::new();
        vars.insert(
            "A".to_string(),
            ExprValue::Series { times: vec![0, 1, 2], vals: vec![Some(1.0), Some(2.0), Some(3.0)] },
        );
        let cmd = ExprCommand::Reduce {
            input: "A".into(),
            reducer: ReducerId::Mean,
            mapper: None,
        };
        match evaluate(&cmd, &vars).unwrap() {
            ExprValue::Number(v) => assert_eq!(v, Some(2.0)),
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_evaluate_math_two_refs() {
        // The defining mixed-datasource case: combine results A and B by refId.
        let mut vars = HashMap::new();
        vars.insert("A".to_string(), ExprValue::Number(Some(10.0)));
        vars.insert("B".to_string(), ExprValue::Number(Some(4.0)));
        let cmd = ExprCommand::MathBinary {
            left: Operand::Ref("A".into()),
            op: "-".into(),
            right: Operand::Ref("B".into()),
        };
        match evaluate(&cmd, &vars).unwrap() {
            ExprValue::Number(v) => assert_eq!(v, Some(6.0)),
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_evaluate_math_ref_and_literal_with_nil() {
        let mut vars = HashMap::new();
        vars.insert("A".to_string(), ExprValue::Number(None));
        let cmd = ExprCommand::MathBinary {
            left: Operand::Ref("A".into()),
            op: "+".into(),
            right: Operand::Lit(5.0),
        };
        match evaluate(&cmd, &vars).unwrap() {
            ExprValue::Number(v) => assert_eq!(v, None),
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_evaluate_threshold() {
        let mut vars = HashMap::new();
        vars.insert("A".to_string(), ExprValue::Number(Some(12.0)));
        let cmd = ExprCommand::Threshold {
            input: "A".into(),
            evaluator: Evaluator::Threshold { typ: "gt".into(), threshold: 10.0 },
        };
        match evaluate(&cmd, &vars).unwrap() {
            ExprValue::Number(v) => assert_eq!(v, Some(1.0)),
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_evaluate_classic_condition_firing() {
        let mut vars = HashMap::new();
        vars.insert(
            "A".to_string(),
            ExprValue::Series { times: vec![0, 1, 2], vals: vec![Some(5.0), Some(5.0), Some(5.0)] },
        );
        let cmd = ExprCommand::ClassicCondition {
            conditions: vec![ClassicCond {
                input: "A".into(),
                reducer: ClassicReducer::Avg,
                evaluator: Evaluator::Threshold { typ: "gt".into(), threshold: 4.0 },
                operator: ConditionOperator::And,
            }],
        };
        match evaluate(&cmd, &vars).unwrap() {
            ExprValue::Number(v) => assert_eq!(v, Some(1.0)),
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_evaluate_classic_condition_nodata() {
        let mut vars = HashMap::new();
        vars.insert(
            "A".to_string(),
            ExprValue::Series { times: vec![0], vals: vec![None] },
        );
        let cmd = ExprCommand::ClassicCondition {
            conditions: vec![ClassicCond {
                input: "A".into(),
                reducer: ClassicReducer::Avg,
                evaluator: Evaluator::Threshold { typ: "gt".into(), threshold: 4.0 },
                operator: ConditionOperator::And,
            }],
        };
        match evaluate(&cmd, &vars).unwrap() {
            ExprValue::Number(v) => assert_eq!(v, None), // no data → nil
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_evaluate_missing_ref_errors() {
        let vars = HashMap::new();
        let cmd = ExprCommand::Reduce { input: "Z".into(), reducer: ReducerId::Sum, mapper: None };
        assert!(evaluate(&cmd, &vars).is_err());
    }
}
