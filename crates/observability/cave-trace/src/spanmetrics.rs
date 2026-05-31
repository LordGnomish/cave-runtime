// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tempo metrics-generator span-metrics processor (RED metrics).
//!
//! Ports grafana/tempo `modules/generator/processor/spanmetrics`
//! (spanmetrics.go + config.go). For every span the processor emits, keyed by the
//! configured dimensions (service / span_name / span_kind / status_code):
//!   - `traces_spanmetrics_calls_total`   — request-rate counter
//!   - `traces_spanmetrics_latency`       — duration histogram (seconds)
//!   - `traces_spanmetrics_size_total`    — span size counter (opt-in)
//!
//! Error rate is captured through the `status_code` dimension (query
//! `calls_total{status_code="STATUS_CODE_ERROR"}`), exactly as Tempo does — there
//! is no separate error counter.
//!
//! This is the Tempo metrics-generator side and is distinct from the in-crate
//! Jaeger SPM (src/spm.rs), which serves the Jaeger `/api/metrics` SPM surface.
//!
//! Upstream: grafana/tempo (Apache-2.0).

use std::collections::BTreeMap;

/// Tempo default latency buckets — `prometheus.ExponentialBuckets(0.002, 2, 14)`
/// (seconds): 0.002 · 2ⁿ for n = 0..13.
pub const DEFAULT_HISTOGRAM_BUCKETS: [f64; 14] = [
    0.002, 0.004, 0.008, 0.016, 0.032, 0.064, 0.128, 0.256, 0.512, 1.024, 2.048, 4.096, 8.192,
    16.384,
];

pub const METRIC_CALLS: &str = "traces_spanmetrics_calls_total";
pub const METRIC_LATENCY: &str = "traces_spanmetrics_latency";
pub const METRIC_SIZE: &str = "traces_spanmetrics_size_total";

/// OTLP status-code string that marks a span as failed.
pub const STATUS_CODE_ERROR: &str = "STATUS_CODE_ERROR";

/// Returns true when a span's OTLP status code marks it as an error.
pub fn is_error_status(status_code: &str) -> bool {
    status_code == STATUS_CODE_ERROR
}

/// Processor configuration (config.go).
#[derive(Debug, Clone)]
pub struct SpanMetricsConfig {
    pub buckets: Vec<f64>,
    /// Emit `traces_spanmetrics_size_total` (Tempo `enable_target_info`-style opt-in).
    pub enable_size: bool,
}

impl Default for SpanMetricsConfig {
    fn default() -> Self {
        Self {
            buckets: DEFAULT_HISTOGRAM_BUCKETS.to_vec(),
            enable_size: false,
        }
    }
}

/// The label set identifying one metric series.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SeriesKey {
    pub service: String,
    pub span_name: String,
    pub span_kind: String,
    pub status_code: String,
}

#[derive(Debug, Clone)]
struct Histogram {
    buckets: Vec<f64>,
    /// Per-bucket (non-cumulative) observation counts; last slot is +Inf overflow.
    counts: Vec<u64>,
    sum: f64,
    count: u64,
}

impl Histogram {
    fn new(buckets: Vec<f64>) -> Self {
        let n = buckets.len() + 1; // +Inf
        Self {
            buckets,
            counts: vec![0; n],
            sum: 0.0,
            count: 0,
        }
    }

    fn observe(&mut self, _value: f64) {
        unimplemented!("RED")
    }

    /// Cumulative count of observations ≤ `le`.
    fn cumulative_le(&self, _le: f64) -> u64 {
        unimplemented!("RED")
    }
}

#[derive(Debug, Clone)]
struct Series {
    calls: u64,
    size_total: u64,
    hist: Histogram,
}

/// Tempo span-metrics processor.
#[derive(Debug, Clone)]
pub struct SpanMetricsProcessor {
    config: SpanMetricsConfig,
    series: BTreeMap<SeriesKey, Series>,
}

impl SpanMetricsProcessor {
    pub fn new(config: SpanMetricsConfig) -> Self {
        Self {
            config,
            series: BTreeMap::new(),
        }
    }

    pub fn with_default() -> Self {
        Self::new(SpanMetricsConfig::default())
    }

    /// Aggregate one span into the calls counter, latency histogram and (when
    /// enabled) the size counter.
    pub fn record_span(
        &mut self,
        _service: &str,
        _span_name: &str,
        _span_kind: &str,
        _status_code: &str,
        _duration_secs: f64,
        _size_bytes: u64,
    ) {
        unimplemented!("RED")
    }

    pub fn calls_total(&self, key: &SeriesKey) -> u64 {
        self.series.get(key).map(|s| s.calls).unwrap_or(0)
    }

    pub fn size_total(&self, key: &SeriesKey) -> u64 {
        self.series.get(key).map(|s| s.size_total).unwrap_or(0)
    }

    pub fn latency_count(&self, key: &SeriesKey) -> u64 {
        self.series.get(key).map(|s| s.hist.count).unwrap_or(0)
    }

    pub fn latency_sum(&self, key: &SeriesKey) -> f64 {
        self.series.get(key).map(|s| s.hist.sum).unwrap_or(0.0)
    }

    /// Cumulative histogram bucket count for series `key` at boundary `le`.
    pub fn latency_bucket(&self, key: &SeriesKey, le: f64) -> u64 {
        self.series
            .get(key)
            .map(|s| s.hist.cumulative_le(le))
            .unwrap_or(0)
    }

    pub fn series_count(&self) -> usize {
        self.series.len()
    }

    /// Prometheus text exposition of all series.
    pub fn expose_prometheus(&self) -> String {
        unimplemented!("RED")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(svc: &str, name: &str, kind: &str, status: &str) -> SeriesKey {
        SeriesKey {
            service: svc.into(),
            span_name: name.into(),
            span_kind: kind.into(),
            status_code: status.into(),
        }
    }

    #[test]
    fn default_buckets_are_tempo_exponential() {
        let c = SpanMetricsConfig::default();
        assert_eq!(c.buckets.len(), 14);
        assert_eq!(c.buckets[0], 0.002);
        assert_eq!(c.buckets[13], 16.384);
        // exponential factor 2
        for w in c.buckets.windows(2) {
            assert!((w[1] / w[0] - 2.0).abs() < 1e-9);
        }
    }

    #[test]
    fn is_error_status_only_for_error_code() {
        assert!(is_error_status("STATUS_CODE_ERROR"));
        assert!(!is_error_status("STATUS_CODE_OK"));
        assert!(!is_error_status("STATUS_CODE_UNSET"));
    }

    #[test]
    fn record_increments_calls_total() {
        let mut p = SpanMetricsProcessor::with_default();
        let k = key("svcA", "GET /x", "SPAN_KIND_SERVER", "STATUS_CODE_OK");
        p.record_span("svcA", "GET /x", "SPAN_KIND_SERVER", "STATUS_CODE_OK", 0.05, 100);
        p.record_span("svcA", "GET /x", "SPAN_KIND_SERVER", "STATUS_CODE_OK", 0.07, 100);
        assert_eq!(p.calls_total(&k), 2);
    }

    #[test]
    fn error_spans_keyed_by_status_dimension() {
        let mut p = SpanMetricsProcessor::with_default();
        p.record_span("svcA", "op", "SPAN_KIND_SERVER", "STATUS_CODE_OK", 0.01, 0);
        p.record_span("svcA", "op", "SPAN_KIND_SERVER", "STATUS_CODE_ERROR", 0.01, 0);
        let ok = key("svcA", "op", "SPAN_KIND_SERVER", "STATUS_CODE_OK");
        let err = key("svcA", "op", "SPAN_KIND_SERVER", "STATUS_CODE_ERROR");
        assert_eq!(p.calls_total(&ok), 1);
        assert_eq!(p.calls_total(&err), 1);
        assert_eq!(p.series_count(), 2);
    }

    #[test]
    fn latency_histogram_count_and_sum() {
        let mut p = SpanMetricsProcessor::with_default();
        let k = key("svcA", "op", "SPAN_KIND_CLIENT", "STATUS_CODE_OK");
        p.record_span("svcA", "op", "SPAN_KIND_CLIENT", "STATUS_CODE_OK", 0.01, 0);
        p.record_span("svcA", "op", "SPAN_KIND_CLIENT", "STATUS_CODE_OK", 0.5, 0);
        assert_eq!(p.latency_count(&k), 2);
        assert!((p.latency_sum(&k) - 0.51).abs() < 1e-9);
    }

    #[test]
    fn latency_buckets_are_cumulative() {
        let mut p = SpanMetricsProcessor::with_default();
        let k = key("svcA", "op", "SPAN_KIND_CLIENT", "STATUS_CODE_OK");
        // 0.01 falls in le=0.016; 0.5 falls in le=0.512
        p.record_span("svcA", "op", "SPAN_KIND_CLIENT", "STATUS_CODE_OK", 0.01, 0);
        p.record_span("svcA", "op", "SPAN_KIND_CLIENT", "STATUS_CODE_OK", 0.5, 0);
        assert_eq!(p.latency_bucket(&k, 0.008), 0); // neither ≤ 0.008
        assert_eq!(p.latency_bucket(&k, 0.016), 1); // only 0.01
        assert_eq!(p.latency_bucket(&k, 0.256), 1); // still only 0.01
        assert_eq!(p.latency_bucket(&k, 0.512), 2); // both
        assert_eq!(p.latency_bucket(&k, 16.384), 2);
    }

    #[test]
    fn size_total_only_when_enabled() {
        let k = key("svcA", "op", "SPAN_KIND_SERVER", "STATUS_CODE_OK");

        let mut off = SpanMetricsProcessor::with_default();
        off.record_span("svcA", "op", "SPAN_KIND_SERVER", "STATUS_CODE_OK", 0.01, 250);
        assert_eq!(off.size_total(&k), 0);

        let mut on = SpanMetricsProcessor::new(SpanMetricsConfig {
            enable_size: true,
            ..Default::default()
        });
        on.record_span("svcA", "op", "SPAN_KIND_SERVER", "STATUS_CODE_OK", 0.01, 250);
        on.record_span("svcA", "op", "SPAN_KIND_SERVER", "STATUS_CODE_OK", 0.01, 50);
        assert_eq!(on.size_total(&k), 300);
    }

    #[test]
    fn prometheus_exposition_has_metric_names_and_labels() {
        let mut p = SpanMetricsProcessor::with_default();
        p.record_span("svcA", "GET /x", "SPAN_KIND_SERVER", "STATUS_CODE_ERROR", 0.05, 0);
        let text = p.expose_prometheus();
        assert!(text.contains("traces_spanmetrics_calls_total"));
        assert!(text.contains("traces_spanmetrics_latency_bucket"));
        assert!(text.contains("traces_spanmetrics_latency_sum"));
        assert!(text.contains("traces_spanmetrics_latency_count"));
        assert!(text.contains("service=\"svcA\""));
        assert!(text.contains("span_name=\"GET /x\""));
        assert!(text.contains("span_kind=\"SPAN_KIND_SERVER\""));
        assert!(text.contains("status_code=\"STATUS_CODE_ERROR\""));
        assert!(text.contains("le=\"+Inf\""));
    }
}
