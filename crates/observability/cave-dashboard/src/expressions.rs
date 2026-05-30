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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
