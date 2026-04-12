//! PromQL built-in functions.

#![allow(dead_code)]

use regex::Regex;
use crate::model::{Labels, Sample};
use super::engine::InstantSample;

/// Compute per-second rate using linear extrapolation.
pub fn rate(samples: &[Sample], range_ms: i64) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let first = samples.first().unwrap();
    let last = samples.last().unwrap();
    let duration_s = range_ms as f64 / 1000.0;
    if duration_s <= 0.0 {
        return None;
    }
    let mut increase = last.value - first.value;
    // Handle counter resets
    if increase < 0.0 {
        increase += last.value;
    }
    // Extrapolation
    let actual_s = (last.timestamp - first.timestamp) as f64 / 1000.0;
    if actual_s <= 0.0 {
        return None;
    }
    Some(increase / actual_s)
}

/// Instant rate using last two samples.
pub fn irate(samples: &[Sample]) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let prev = samples[samples.len() - 2];
    let last = samples[samples.len() - 1];
    let duration_s = (last.timestamp - prev.timestamp) as f64 / 1000.0;
    if duration_s <= 0.0 {
        return None;
    }
    let mut increase = last.value - prev.value;
    if increase < 0.0 {
        increase += last.value;
    }
    Some(increase / duration_s)
}

/// Total increase over the range.
pub fn increase(samples: &[Sample], range_ms: i64) -> Option<f64> {
    rate(samples, range_ms).map(|r| r * range_ms as f64 / 1000.0)
}

/// delta: last - first.
pub fn delta(samples: &[Sample]) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    Some(samples.last().unwrap().value - samples.first().unwrap().value)
}

/// deriv: slope via simple linear regression.
pub fn deriv(samples: &[Sample]) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let (slope, _) = linear_regression(samples);
    Some(slope)
}

/// predict_linear: linear regression and predict duration_s into the future.
pub fn predict_linear(samples: &[Sample], duration_s: f64) -> Option<f64> {
    if samples.len() < 2 {
        return None;
    }
    let (slope, intercept) = linear_regression(samples);
    let last_ts = samples.last().unwrap().timestamp as f64 / 1000.0;
    Some(intercept + slope * (last_ts + duration_s))
}

fn linear_regression(samples: &[Sample]) -> (f64, f64) {
    let n = samples.len() as f64;
    let sum_x: f64 = samples.iter().map(|s| s.timestamp as f64 / 1000.0).sum();
    let sum_y: f64 = samples.iter().map(|s| s.value).sum();
    let sum_xy: f64 = samples.iter().map(|s| s.timestamp as f64 / 1000.0 * s.value).sum();
    let sum_xx: f64 = samples.iter().map(|s| (s.timestamp as f64 / 1000.0).powi(2)).sum();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < f64::EPSILON {
        return (0.0, sum_y / n);
    }
    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    let intercept = (sum_y - slope * sum_x) / n;
    (slope, intercept)
}

/// Compute histogram quantile from cumulative bucket list: (upper_bound, count).
pub fn histogram_quantile(q: f64, buckets: &[(f64, f64)]) -> f64 {
    if buckets.is_empty() {
        return f64::NAN;
    }
    let mut sorted = buckets.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let total = sorted.last().map(|(_, c)| *c).unwrap_or(0.0);
    if total == 0.0 {
        return f64::NAN;
    }
    let rank = q * total;
    let mut prev_upper = 0.0_f64;
    let mut prev_count = 0.0_f64;
    for &(upper, count) in &sorted {
        if count >= rank {
            let bucket_width = upper - prev_upper;
            let bucket_count = count - prev_count;
            if bucket_count == 0.0 {
                return upper;
            }
            return prev_upper + bucket_width * (rank - prev_count) / bucket_count;
        }
        prev_upper = upper;
        prev_count = count;
    }
    sorted.last().map(|(u, _)| *u).unwrap_or(f64::NAN)
}

/// Count value changes.
pub fn changes(samples: &[Sample]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    samples.windows(2).filter(|w| w[0].value != w[1].value).count() as f64
}

/// label_replace: apply regex replacement to a label.
pub fn label_replace(
    labels: &Labels,
    dst_label: &str,
    replacement: &str,
    src_label: &str,
    regex: &str,
) -> Labels {
    let full_regex = format!("^(?:{})$", regex);
    let re = match Regex::new(&full_regex) {
        Ok(r) => r,
        Err(_) => return labels.clone(),
    };
    let src_value = labels.get(src_label).unwrap_or("");
    let new_value = if re.is_match(src_value) {
        re.replace(src_value, replacement).to_string()
    } else {
        return labels.clone();
    };
    let mut m = labels.0.clone();
    if new_value.is_empty() {
        m.remove(dst_label);
    } else {
        m.insert(dst_label.to_string(), new_value);
    }
    Labels(m)
}

pub fn sort_asc(mut v: Vec<InstantSample>) -> Vec<InstantSample> {
    v.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
    v
}

pub fn topk(k: usize, mut v: Vec<InstantSample>) -> Vec<InstantSample> {
    v.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(k);
    v
}

pub fn bottomk(k: usize, mut v: Vec<InstantSample>) -> Vec<InstantSample> {
    v.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(k);
    v
}

pub fn absent(v: &[InstantSample]) -> Vec<InstantSample> {
    if v.is_empty() {
        vec![InstantSample {
            labels: Labels::default(),
            value: 1.0,
            timestamp: 0,
        }]
    } else {
        vec![]
    }
}
