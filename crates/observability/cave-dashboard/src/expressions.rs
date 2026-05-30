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
}
