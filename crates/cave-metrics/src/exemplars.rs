// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Exemplar storage + native histogram engine.
//!
//! upstream: prometheus/prometheus — pkg/exemplars/ + pkg/storage/native-histogram/
//!
//! An exemplar is a (trace-id, value, timestamp, labels) triple attached
//! to a metric sample so PromQL queries can hop to the underlying trace
//! that produced an outlier. Upstream keeps a fixed-size ring per
//! series so memory stays bounded; we port that ring plus the
//! native-histogram bucket arithmetic Prometheus uses to compress
//! histogram observations into log-spaced "spans".

use std::collections::HashMap;

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Exemplar {
    pub timestamp_ms: i64,
    pub value: f64,
    pub trace_id: String,
    pub span_id: Option<String>,
    pub labels: HashMap<String, String>,
}

/// Fixed-capacity ring of exemplars per series.
#[derive(Debug, Clone)]
pub struct ExemplarRing {
    capacity: usize,
    buf: Vec<Exemplar>,
}

impl ExemplarRing {
    pub fn new(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), buf: Vec::with_capacity(capacity.max(1)) }
    }

    /// Push a new exemplar. Drops the oldest if at capacity. Exemplars
    /// older than the newest by `keep_window_ms` are also evicted on
    /// every push.
    pub fn push(&mut self, e: Exemplar, keep_window_ms: i64) {
        if let Some(last) = self.buf.last() {
            let newest_ts = last.timestamp_ms.max(e.timestamp_ms);
            let cutoff = newest_ts - keep_window_ms;
            self.buf.retain(|x| x.timestamp_ms >= cutoff);
        }
        if self.buf.len() >= self.capacity {
            self.buf.remove(0);
        }
        self.buf.push(e);
    }

    pub fn len(&self) -> usize { self.buf.len() }
    pub fn is_empty(&self) -> bool { self.buf.is_empty() }

    /// Return exemplars in the given [start_ms, end_ms] window (inclusive).
    pub fn in_range(&self, start_ms: i64, end_ms: i64) -> Vec<&Exemplar> {
        self.buf
            .iter()
            .filter(|e| e.timestamp_ms >= start_ms && e.timestamp_ms <= end_ms)
            .collect()
    }
}

/// Native histogram bucket — Prometheus uses log-spaced "schema" indices.
/// Bucket `index` covers values in `(base^index, base^(index+1)]` where
/// `base = 2^(2^-schema)`.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeHistogram {
    pub schema: i32,
    pub zero_threshold: f64,
    pub zero_count: u64,
    pub count: u64,
    pub sum: f64,
    pub positive_buckets: HashMap<i32, u64>,
    pub negative_buckets: HashMap<i32, u64>,
}

impl NativeHistogram {
    pub fn new(schema: i32) -> Self {
        Self {
            schema,
            zero_threshold: 0.0,
            zero_count: 0,
            count: 0,
            sum: 0.0,
            positive_buckets: HashMap::new(),
            negative_buckets: HashMap::new(),
        }
    }

    /// Schema base.  schema=0 ⇒ base=2, schema=2 ⇒ base=2^(1/4)≈1.189, etc.
    pub fn base(&self) -> f64 {
        // base = 2^(2^-schema)
        let exp = 2f64.powi(-self.schema);
        2f64.powf(exp)
    }

    /// Compute the bucket index for an observed value. Always pairs with
    /// `(base^index, base^(index+1)]`.
    pub fn bucket_index(&self, v: f64) -> i32 {
        let base = self.base();
        // ceil(log_base(|v|)) - 1
        ((v.abs().ln() / base.ln()).ceil() as i32) - 1
    }

    pub fn observe(&mut self, value: f64) {
        self.count += 1;
        self.sum += value;
        if value.abs() <= self.zero_threshold {
            self.zero_count += 1;
            return;
        }
        let idx = self.bucket_index(value);
        if value > 0.0 {
            *self.positive_buckets.entry(idx).or_insert(0) += 1;
        } else {
            *self.negative_buckets.entry(idx).or_insert(0) += 1;
        }
    }

    /// Merge another histogram of the same schema into this one. Returns
    /// `Err` if the schemas differ — upstream forbids cross-schema merges.
    pub fn merge(&mut self, other: &Self) -> Result<(), &'static str> {
        if self.schema != other.schema {
            return Err("schema mismatch");
        }
        self.count += other.count;
        self.sum += other.sum;
        self.zero_count += other.zero_count;
        for (i, c) in &other.positive_buckets {
            *self.positive_buckets.entry(*i).or_insert(0) += *c;
        }
        for (i, c) in &other.negative_buckets {
            *self.negative_buckets.entry(*i).or_insert(0) += *c;
        }
        Ok(())
    }

    /// Estimate a quantile (0..1) from the positive buckets only.  Returns
    /// `f64::NAN` if no positive observations exist.
    pub fn quantile(&self, q: f64) -> f64 {
        if self.positive_buckets.is_empty() || !(0.0..=1.0).contains(&q) {
            return f64::NAN;
        }
        let target = (self.count as f64) * q;
        let mut indices: Vec<i32> = self.positive_buckets.keys().copied().collect();
        indices.sort();
        let mut acc = self.zero_count as f64;
        let base = self.base();
        for idx in indices {
            let c = self.positive_buckets[&idx] as f64;
            if acc + c >= target {
                let frac = (target - acc) / c;
                let lo = base.powi(idx);
                let hi = base.powi(idx + 1);
                return lo + frac * (hi - lo);
            }
            acc += c;
        }
        // Fall through to the upper edge of the highest bucket.
        let max_idx = *self.positive_buckets.keys().max().unwrap();
        base.powi(max_idx + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(ts: i64, v: f64) -> Exemplar {
        Exemplar {
            timestamp_ms: ts,
            value: v,
            trace_id: format!("trace-{}", ts),
            span_id: None,
            labels: HashMap::new(),
        }
    }

    // ─── ExemplarRing ───────────────────────────────────────────────────

    #[test]
    fn ring_drops_oldest_when_capacity_reached() {
        let mut r = ExemplarRing::new(2);
        r.push(ex(1, 1.0), 60_000);
        r.push(ex(2, 2.0), 60_000);
        r.push(ex(3, 3.0), 60_000);
        assert_eq!(r.len(), 2);
        assert!(r.in_range(0, 100).iter().all(|e| e.timestamp_ms >= 2));
    }

    #[test]
    fn ring_evicts_outside_keep_window() {
        let mut r = ExemplarRing::new(8);
        r.push(ex(1_000, 1.0), 5_000);
        r.push(ex(2_000, 1.0), 5_000);
        r.push(ex(10_000, 1.0), 5_000); // window cutoff = 5_000
        assert_eq!(r.len(), 1);
        assert_eq!(r.in_range(0, 20_000)[0].timestamp_ms, 10_000);
    }

    #[test]
    fn ring_in_range_filters_inclusive() {
        let mut r = ExemplarRing::new(10);
        r.push(ex(1, 1.0), 60_000);
        r.push(ex(5, 2.0), 60_000);
        r.push(ex(10, 3.0), 60_000);
        let v: Vec<_> = r.in_range(2, 9).iter().map(|e| e.timestamp_ms).collect();
        assert_eq!(v, vec![5]);
    }

    #[test]
    fn ring_empty_returns_no_exemplars() {
        let r = ExemplarRing::new(4);
        assert!(r.is_empty());
        assert!(r.in_range(0, 100).is_empty());
    }

    // ─── NativeHistogram ────────────────────────────────────────────────

    #[test]
    fn native_hist_zero_count_when_zero_threshold_observed() {
        let mut h = NativeHistogram::new(0);
        h.zero_threshold = 0.5;
        h.observe(0.2);
        h.observe(0.4);
        assert_eq!(h.zero_count, 2);
        assert!(h.positive_buckets.is_empty());
    }

    #[test]
    fn native_hist_positive_observations_increment_correct_bucket() {
        let mut h = NativeHistogram::new(0); // base = 2
        h.observe(3.0); // log2(3) ≈ 1.58 → ceil=2 → idx = 1
        h.observe(3.5);
        assert_eq!(h.positive_buckets.get(&1), Some(&2));
    }

    #[test]
    fn native_hist_merge_rejects_different_schema() {
        let mut a = NativeHistogram::new(0);
        let b = NativeHistogram::new(1);
        assert!(a.merge(&b).is_err());
    }

    #[test]
    fn native_hist_merge_sums_counts_and_buckets() {
        let mut a = NativeHistogram::new(0);
        a.observe(3.0);
        let mut b = NativeHistogram::new(0);
        b.observe(3.5);
        a.merge(&b).unwrap();
        assert_eq!(a.count, 2);
        assert_eq!(a.positive_buckets.get(&1), Some(&2));
    }

    #[test]
    fn native_hist_quantile_falls_within_bucket_range() {
        let mut h = NativeHistogram::new(0);
        for _ in 0..10 {
            h.observe(3.0); // bucket idx=1 → (2,4]
        }
        let q = h.quantile(0.5);
        assert!(q >= 2.0 && q <= 4.0, "got q={}", q);
    }

    #[test]
    fn native_hist_quantile_empty_returns_nan() {
        let h = NativeHistogram::new(0);
        assert!(h.quantile(0.5).is_nan());
    }

    #[test]
    fn native_hist_sum_and_count_track_observations() {
        let mut h = NativeHistogram::new(0);
        h.observe(1.0);
        h.observe(2.0);
        h.observe(3.0);
        assert_eq!(h.count, 3);
        assert!((h.sum - 6.0).abs() < 1e-9);
    }
}
