// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! All built-in PromQL functions.

use crate::error::{MetricsError, Result};
use crate::model::{Labels, Sample};

// ─── Rate family ─────────────────────────────────────────────────────────────

/// rate(): per-second rate with counter reset detection + extrapolation.
pub fn rate(samples: &[Sample], range_ms: i64) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let first = samples[0];
    let last = samples[samples.len() - 1];
    let dur_s = (last.timestamp_ms - first.timestamp_ms) as f64 / 1000.0;
    if dur_s <= 0.0 {
        return None;
    }

    let mut delta = last.value - first.value;
    // Detect counter resets
    for w in samples.windows(2) {
        if w[1].value < w[0].value {
            delta += w[0].value;
        }
    }

    // Extrapolation
    let range_s = range_ms as f64 / 1000.0;
    let extrapolation = range_s / dur_s;
    let extrapolated = delta * extrapolation.min(1.1);

    Some(extrapolated / range_s)
}

/// irate(): instant rate using the last two samples.
pub fn irate(samples: &[Sample]) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let prev = &samples[samples.len() - 2];
    let last = &samples[samples.len() - 1];
    let dur_s = (last.timestamp_ms - prev.timestamp_ms) as f64 / 1000.0;
    if dur_s <= 0.0 {
        return None;
    }
    let delta = if last.value >= prev.value {
        last.value - prev.value
    } else {
        last.value
    };
    Some(delta / dur_s)
}

/// increase(): total increase in a counter over the range.
pub fn increase(samples: &[Sample], range_ms: i64) -> Option<f64> {
    rate(samples, range_ms).map(|r| r * (range_ms as f64 / 1000.0))
}

/// delta(): difference between last and first sample.
pub fn delta(samples: &[Sample], range_ms: i64) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let first = samples[0];
    let last = samples[samples.len() - 1];
    let dur_s = (last.timestamp_ms - first.timestamp_ms) as f64 / 1000.0;
    if dur_s <= 0.0 {
        return None;
    }
    let range_s = range_ms as f64 / 1000.0;
    Some((last.value - first.value) * (range_s / dur_s))
}

/// idelta(): instant delta — difference between last two samples.
pub fn idelta(samples: &[Sample]) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let n = samples.len();
    Some(samples[n - 1].value - samples[n - 2].value)
}

/// deriv(): least-squares derivative (per second).
pub fn deriv(samples: &[Sample]) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let n = samples.len() as f64;
    let sum_x: f64 = samples.iter().map(|s| s.timestamp_ms as f64 / 1000.0).sum();
    let sum_y: f64 = samples.iter().map(|s| s.value).sum();
    let sum_xy: f64 = samples
        .iter()
        .map(|s| (s.timestamp_ms as f64 / 1000.0) * s.value)
        .sum();
    let sum_xx: f64 = samples
        .iter()
        .map(|s| (s.timestamp_ms as f64 / 1000.0).powi(2))
        .sum();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom == 0.0 {
        return None;
    }
    Some((n * sum_xy - sum_x * sum_y) / denom)
}

/// predict_linear(): linear extrapolation t seconds into the future.
pub fn predict_linear(samples: &[Sample], t: f64) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let slope = deriv(samples)?;
    let last = samples.last()?;
    let intercept = last.value - slope * (last.timestamp_ms as f64 / 1000.0);
    let predict_at = last.timestamp_ms as f64 / 1000.0 + t;
    Some(slope * predict_at + intercept)
}

/// resets(): count counter resets.
pub fn resets(samples: &[Sample]) -> f64 {
    samples
        .windows(2)
        .filter(|w| w[1].value < w[0].value)
        .count() as f64
}

/// changes(): count value changes.
pub fn changes(samples: &[Sample]) -> f64 {
    samples
        .windows(2)
        .filter(|w| w[0].value != w[1].value)
        .count() as f64
}

// ─── Over-time aggregations ──────────────────────────────────────────────────

pub fn avg_over_time(samples: &[Sample]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    Some(samples.iter().map(|s| s.value).sum::<f64>() / samples.len() as f64)
}

pub fn min_over_time(samples: &[Sample]) -> Option<f64> {
    samples.iter().map(|s| s.value).reduce(f64::min)
}

pub fn max_over_time(samples: &[Sample]) -> Option<f64> {
    samples.iter().map(|s| s.value).reduce(f64::max)
}

pub fn sum_over_time(samples: &[Sample]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    Some(samples.iter().map(|s| s.value).sum())
}

pub fn count_over_time(samples: &[Sample]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    Some(samples.len() as f64)
}

pub fn stddev_over_time(samples: &[Sample]) -> Option<f64> {
    let mean = avg_over_time(samples)?;
    let var = samples
        .iter()
        .map(|s| (s.value - mean).powi(2))
        .sum::<f64>()
        / samples.len() as f64;
    Some(var.sqrt())
}

pub fn stdvar_over_time(samples: &[Sample]) -> Option<f64> {
    let mean = avg_over_time(samples)?;
    Some(
        samples
            .iter()
            .map(|s| (s.value - mean).powi(2))
            .sum::<f64>()
            / samples.len() as f64,
    )
}

pub fn last_over_time(samples: &[Sample]) -> Option<f64> {
    samples.last().map(|s| s.value)
}

pub fn present_over_time(samples: &[Sample]) -> Option<f64> {
    if samples.is_empty() { None } else { Some(1.0) }
}

pub fn quantile_over_time(q: f64, samples: &[Sample]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut vals: Vec<f64> = samples.iter().map(|s| s.value).collect();
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(quantile_sorted(q, &vals))
}

pub fn mad_over_time(samples: &[Sample]) -> Option<f64> {
    let median = quantile_over_time(0.5, samples)?;
    let mut diffs: Vec<f64> = samples.iter().map(|s| (s.value - median).abs()).collect();
    diffs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(quantile_sorted(0.5, &diffs))
}

// ─── Histogram ───────────────────────────────────────────────────────────────

/// histogram_quantile(): compute quantile from histogram buckets.
/// Expects `buckets`: Vec<(le_value, cumulative_count)>, sorted by le.
pub fn histogram_quantile(q: f64, mut buckets: Vec<(f64, f64)>) -> f64 {
    if buckets.is_empty() {
        return f64::NAN;
    }
    buckets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let total = buckets.last().map(|b| b.1).unwrap_or(0.0);
    if total <= 0.0 {
        return f64::NAN;
    }
    if q < 0.0 {
        return f64::NEG_INFINITY;
    }
    if q > 1.0 {
        return f64::INFINITY;
    }

    let rank = q * total;
    let mut prev_le = 0.0_f64;
    let mut prev_count = 0.0_f64;

    for (le, count) in &buckets {
        if *count >= rank {
            // Linear interpolation
            if *le == f64::INFINITY {
                return prev_le;
            }
            let bucket_count = count - prev_count;
            let offset = if bucket_count == 0.0 {
                0.0
            } else {
                (rank - prev_count) / bucket_count
            };
            return prev_le + (le - prev_le) * offset;
        }
        prev_le = *le;
        prev_count = *count;
    }
    f64::INFINITY
}

// ─── Label manipulation ──────────────────────────────────────────────────────

pub fn label_replace(
    labels: &Labels,
    dst_label: &str,
    replacement: &str,
    src_label: &str,
    regex: &str,
) -> Result<Labels> {
    let anchored = format!("^(?:{})$", regex);
    let re = regex::Regex::new(&anchored).map_err(|e| MetricsError::Parse(e.to_string()))?;
    let src_val = labels.get(src_label).unwrap_or("");
    let mut out = labels.clone();
    if re.is_match(src_val) {
        let replaced = re.replace(src_val, replacement).to_string();
        if replaced.is_empty() {
            out.0.remove(dst_label);
        } else {
            out.insert(dst_label, replaced);
        }
    }
    Ok(out)
}

pub fn label_join(
    labels: &Labels,
    dst_label: &str,
    separator: &str,
    src_labels: &[&str],
) -> Labels {
    let joined: Vec<&str> = src_labels.iter().filter_map(|l| labels.get(l)).collect();
    let val = joined.join(separator);
    let mut out = labels.clone();
    out.insert(dst_label, val);
    out
}

// ─── Math functions ──────────────────────────────────────────────────────────

pub fn clamp(v: f64, min: f64, max: f64) -> f64 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

// ─── Time functions ──────────────────────────────────────────────────────────

pub fn timestamp_to_day_of_month(ts_ms: i64) -> f64 {
    use chrono::{DateTime, Datelike, Utc};
    let dt = DateTime::<Utc>::from_timestamp_millis(ts_ms).unwrap_or_default();
    dt.day() as f64
}

pub fn timestamp_to_day_of_week(ts_ms: i64) -> f64 {
    use chrono::{DateTime, Datelike, Utc};
    let dt = DateTime::<Utc>::from_timestamp_millis(ts_ms).unwrap_or_default();
    dt.weekday().num_days_from_sunday() as f64
}

pub fn timestamp_to_day_of_year(ts_ms: i64) -> f64 {
    use chrono::{DateTime, Datelike, Utc};
    let dt = DateTime::<Utc>::from_timestamp_millis(ts_ms).unwrap_or_default();
    dt.ordinal() as f64
}

pub fn days_in_month(ts_ms: i64) -> f64 {
    use chrono::{DateTime, Datelike, Utc};
    let dt = DateTime::<Utc>::from_timestamp_millis(ts_ms).unwrap_or_default();
    let month = dt.month();
    let year = dt.year();
    // Days in month
    let next_month = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    let this_month = chrono::NaiveDate::from_ymd_opt(year, month, 1);
    match (this_month, next_month) {
        (Some(t), Some(n)) => (n - t).num_days() as f64,
        _ => 30.0,
    }
}

pub fn timestamp_to_hour(ts_ms: i64) -> f64 {
    use chrono::{DateTime, Timelike, Utc};
    DateTime::<Utc>::from_timestamp_millis(ts_ms)
        .map(|d| d.hour() as f64)
        .unwrap_or(0.0)
}

pub fn timestamp_to_minute(ts_ms: i64) -> f64 {
    use chrono::{DateTime, Timelike, Utc};
    DateTime::<Utc>::from_timestamp_millis(ts_ms)
        .map(|d| d.minute() as f64)
        .unwrap_or(0.0)
}

pub fn timestamp_to_month(ts_ms: i64) -> f64 {
    use chrono::{DateTime, Datelike, Utc};
    DateTime::<Utc>::from_timestamp_millis(ts_ms)
        .map(|d| d.month() as f64)
        .unwrap_or(0.0)
}

pub fn timestamp_to_year(ts_ms: i64) -> f64 {
    use chrono::{DateTime, Datelike, Utc};
    DateTime::<Utc>::from_timestamp_millis(ts_ms)
        .map(|d| d.year() as f64)
        .unwrap_or(0.0)
}

// ─── Utilities ───────────────────────────────────────────────────────────────

/// Quantile over a pre-sorted slice (0 ≤ q ≤ 1).
pub fn quantile_sorted(q: f64, sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    if q <= 0.0 {
        return sorted[0];
    }
    if q >= 1.0 {
        return sorted[sorted.len() - 1];
    }
    let rank = q * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let frac = rank - lo as f64;
    sorted[lo] + frac * (sorted[hi] - sorted[lo])
}

/// Sort and deduplicate (labels, value) pairs by value descending.
pub fn topk(k: usize, mut pairs: Vec<(Labels, f64)>) -> Vec<(Labels, f64)> {
    pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    pairs.truncate(k);
    pairs
}

/// Sort and deduplicate by value ascending.
pub fn bottomk(k: usize, mut pairs: Vec<(Labels, f64)>) -> Vec<(Labels, f64)> {
    pairs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    pairs.truncate(k);
    pairs
}

pub fn sort_asc(mut pairs: Vec<(Labels, f64)>) -> Vec<(Labels, f64)> {
    pairs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    pairs
}

pub fn sort_desc(mut pairs: Vec<(Labels, f64)>) -> Vec<(Labels, f64)> {
    pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    pairs
}

// ─── Holt-Winters / double exponential smoothing ──────────────────────────────

fn calc_trend_value(i: usize, tf: f64, s0: f64, s1: f64, b: f64) -> f64 {
    if i == 0 {
        return b;
    }
    tf * (s1 - s0) + (1.0 - tf) * b
}

/// double_exponential_smoothing(): Holt-Winters double exponential smoothing.
/// `sf` is the smoothing factor and `tf` the trend factor (both in (0,1)).
/// Renamed from `holt_winters` in Prometheus 3.0 (promql/functions.go
/// `funcDoubleExponentialSmoothing`). Needs at least two samples.
pub fn double_exponential_smoothing(samples: &[Sample], sf: f64, tf: f64) -> Option<f64> {
    let l = samples.len();
    if l < 2 {
        return None;
    }
    let mut s0 = 0.0_f64;
    let mut s1 = samples[0].value;
    let mut b = samples[1].value - samples[0].value;
    for i in 1..l {
        let x = sf * samples[i].value;
        b = calc_trend_value(i - 1, tf, s0, s1, b);
        let y = (1.0 - sf) * (s1 + b);
        s0 = s1;
        s1 = x + y;
    }
    Some(s1)
}

// ─── Timestamp-of extrema (Prometheus #15232) ─────────────────────────────────

/// ts_of_max_over_time(): timestamp (seconds) of the maximum-valued sample.
/// Ties resolve to the earliest such sample.
pub fn ts_of_max_over_time(samples: &[Sample]) -> Option<f64> {
    let mut best: Option<&Sample> = None;
    for s in samples {
        match best {
            Some(b) if s.value <= b.value => {}
            _ => best = Some(s),
        }
    }
    best.map(|s| s.timestamp_ms as f64 / 1000.0)
}

/// ts_of_min_over_time(): timestamp (seconds) of the minimum-valued sample.
/// Ties resolve to the earliest such sample.
pub fn ts_of_min_over_time(samples: &[Sample]) -> Option<f64> {
    let mut best: Option<&Sample> = None;
    for s in samples {
        match best {
            Some(b) if s.value >= b.value => {}
            _ => best = Some(s),
        }
    }
    best.map(|s| s.timestamp_ms as f64 / 1000.0)
}

/// ts_of_last_over_time(): timestamp (seconds) of the last sample in the range.
pub fn ts_of_last_over_time(samples: &[Sample]) -> Option<f64> {
    samples.last().map(|s| s.timestamp_ms as f64 / 1000.0)
}

// ─── Sort by label (Prometheus #11299) ────────────────────────────────────────

fn full_label_cmp(a: &Labels, b: &Labels) -> std::cmp::Ordering {
    let mut ax: Vec<(String, String)> =
        a.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut bx: Vec<(String, String)> =
        b.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    ax.sort();
    bx.sort();
    ax.cmp(&bx)
}

/// sort_by_label(): order an instant vector by the given label value(s),
/// ascending; ties fall back to a full-label-set comparison so the order is
/// deterministic. `desc` reverses the whole comparison. Port of
/// promql/functions.go `funcSortByLabel` / `funcSortByLabelDesc`.
pub fn sort_by_label(
    mut pairs: Vec<(Labels, f64)>,
    sort_labels: &[&str],
    desc: bool,
) -> Vec<(Labels, f64)> {
    pairs.sort_by(|a, b| {
        let mut ord = std::cmp::Ordering::Equal;
        for &lbl in sort_labels {
            let va = a.0.get(lbl).unwrap_or("");
            let vb = b.0.get(lbl).unwrap_or("");
            ord = va.cmp(vb);
            if ord != std::cmp::Ordering::Equal {
                break;
            }
        }
        if ord == std::cmp::Ordering::Equal {
            ord = full_label_cmp(&a.0, &b.0);
        }
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
    pairs
}
