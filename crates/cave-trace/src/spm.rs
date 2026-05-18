// SPDX-License-Identifier: AGPL-3.0-or-later
//! Service Performance Monitoring (SPM) — RED metrics derived from traces.
//!
//! Computes Request rate, Error rate, Duration histograms per (service, operation)
//! over rolling time windows, similar to Jaeger's SPM / Tempo metrics-generator.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::types::{Span, SpanStatus, TagValue};

// ─── Metric types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanMetric {
    pub service: String,
    pub operation: String,
    pub span_kind: String,
    /// Spans observed in current window.
    pub calls: u64,
    /// Error spans in current window.
    pub errors: u64,
    /// Sum of durations in nanoseconds.
    pub duration_sum_ns: u64,
    /// Minimum duration in ns.
    pub min_duration_ns: u64,
    /// Maximum duration in ns.
    pub max_duration_ns: u64,
    /// Histogram buckets (cumulative counts, upper bound ns).
    pub histogram: Vec<(u64, u64)>,
    /// Window start epoch ns.
    pub window_start_ns: u64,
    /// Window duration in seconds.
    pub window_secs: u64,
}

impl SpanMetric {
    pub fn request_rate(&self) -> f64 {
        if self.window_secs == 0 { 0.0 } else { self.calls as f64 / self.window_secs as f64 }
    }

    pub fn error_rate(&self) -> f64 {
        if self.window_secs == 0 { 0.0 } else { self.errors as f64 / self.window_secs as f64 }
    }

    pub fn error_fraction(&self) -> f64 {
        if self.calls == 0 { 0.0 } else { self.errors as f64 / self.calls as f64 }
    }

    pub fn avg_duration_ms(&self) -> f64 {
        if self.calls == 0 { 0.0 } else { self.duration_sum_ns as f64 / self.calls as f64 / 1_000_000.0 }
    }
}

// ─── Prometheus-compatible text format ─────────────────────────────────────

/// Format metrics as Prometheus text (for /api/metrics compatibility).
pub fn to_prometheus(metrics: &[SpanMetric]) -> String {
    let mut out = String::new();
    for m in metrics {
        let labels = format!(
            r#"service="{}",operation="{}""#,
            escape_prom_label(&m.service),
            escape_prom_label(&m.operation)
        );
        out.push_str(&format!(
            "traces_spanmetrics_calls_total{{{}}} {}\n",
            labels, m.calls
        ));
        out.push_str(&format!(
            "traces_spanmetrics_errors_total{{{}}} {}\n",
            labels, m.errors
        ));
        out.push_str(&format!(
            "traces_spanmetrics_duration_sum_ns{{{}}} {}\n",
            labels, m.duration_sum_ns
        ));
        for (le_ns, count) in &m.histogram {
            let le_label = if *le_ns == u64::MAX {
                "+Inf".to_owned()
            } else {
                format!("{}", le_ns)
            };
            out.push_str(&format!(
                "traces_spanmetrics_duration_bucket{{{},le=\"{}\"}} {}\n",
                labels, le_label, count
            ));
        }
    }
    out
}

fn escape_prom_label(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

// ─── SPM registry ──────────────────────────────────────────────────────────

/// Accumulates span metrics over a rolling window.
pub struct SpmRegistry {
    window: RwLock<WindowState>,
    window_secs: u64,
}

struct WindowState {
    started_at_ns: u64,
    buckets: HashMap<(String, String), BucketAccum>,
}

#[derive(Default)]
struct BucketAccum {
    calls: u64,
    errors: u64,
    dur_sum: u64,
    dur_min: u64,
    dur_max: u64,
    histogram: [u64; 14], // matches LATENCY_BUCKETS_NS
}

const BUCKETS_NS: [u64; 14] = [
    500_000, 1_000_000, 2_500_000, 5_000_000, 10_000_000,
    25_000_000, 50_000_000, 100_000_000, 250_000_000, 500_000_000,
    1_000_000_000, 2_500_000_000, 5_000_000_000, u64::MAX,
];

impl SpmRegistry {
    pub fn new(window_secs: u64) -> Self {
        SpmRegistry {
            window: RwLock::new(WindowState {
                started_at_ns: now_ns(),
                buckets: HashMap::new(),
            }),
            window_secs,
        }
    }

    /// Record a batch of spans.
    pub fn record_spans(&self, spans: &[Span]) {
        let mut state = self.window.write().unwrap();

        for span in spans {
            let key = (span.service_name.clone(), span.operation_name.clone());
            let entry = state.buckets.entry(key).or_default();

            entry.calls += 1;
            if span.has_error() { entry.errors += 1; }
            entry.dur_sum += span.duration_ns;
            if entry.dur_min == 0 || span.duration_ns < entry.dur_min {
                entry.dur_min = span.duration_ns;
            }
            if span.duration_ns > entry.dur_max {
                entry.dur_max = span.duration_ns;
            }
            for (i, &bucket_le) in BUCKETS_NS.iter().enumerate() {
                if span.duration_ns <= bucket_le {
                    entry.histogram[i] += 1;
                    break;
                }
            }
        }
    }

    /// Get current window metrics.
    pub fn snapshot(&self) -> Vec<SpanMetric> {
        let state = self.window.read().unwrap();
        let window_start_ns = state.started_at_ns;

        state.buckets.iter().map(|((svc, op), acc)| {
            let histogram: Vec<(u64, u64)> = BUCKETS_NS.iter().zip(acc.histogram.iter())
                .map(|(&le, &count)| (le, count))
                .collect();

            SpanMetric {
                service: svc.clone(),
                operation: op.clone(),
                span_kind: "SPAN_KIND_UNSPECIFIED".into(),
                calls: acc.calls,
                errors: acc.errors,
                duration_sum_ns: acc.dur_sum,
                min_duration_ns: acc.dur_min,
                max_duration_ns: acc.dur_max,
                histogram,
                window_start_ns,
                window_secs: self.window_secs,
            }
        }).collect()
    }

    /// Rotate window (call periodically to reset counters).
    pub fn rotate(&self) {
        let mut state = self.window.write().unwrap();
        state.started_at_ns = now_ns();
        state.buckets.clear();
    }
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

// ─── Jaeger SPM response types ─────────────────────────────────────────────

/// Jaeger /api/metrics response (subset of MetricsQueryService).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsResponse {
    pub metrics: Vec<MetricFamily>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricFamily {
    pub name: String,
    pub r#type: String,
    pub help: String,
    pub metric_points: Vec<MetricPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricPoint {
    pub labels: Vec<Label>,
    pub gauge_value: Option<f64>,
    pub sum_value: Option<f64>,
    pub histogram_value: Option<HistogramValue>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistogramValue {
    pub sample_count: u64,
    pub sample_sum: f64,
    pub buckets: Vec<HistogramBucketValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramBucketValue {
    pub upper_bound: f64,
    pub cumulative_count: u64,
}

/// Convert SPM snapshot to Jaeger MetricsResponse.
pub fn to_jaeger_metrics(metrics: &[SpanMetric]) -> MetricsResponse {
    let now = chrono::Utc::now().to_rfc3339();

    let mut call_points: Vec<MetricPoint> = Vec::new();
    let mut err_points: Vec<MetricPoint> = Vec::new();
    let mut dur_points: Vec<MetricPoint> = Vec::new();

    for m in metrics {
        let labels = vec![
            Label { name: "service".into(), value: m.service.clone() },
            Label { name: "operation".into(), value: m.operation.clone() },
            Label { name: "span_kind".into(), value: m.span_kind.clone() },
        ];

        call_points.push(MetricPoint {
            labels: labels.clone(),
            gauge_value: Some(m.request_rate()),
            sum_value: None,
            histogram_value: None,
            timestamp: now.clone(),
        });

        err_points.push(MetricPoint {
            labels: labels.clone(),
            gauge_value: Some(m.error_rate()),
            sum_value: None,
            histogram_value: None,
            timestamp: now.clone(),
        });

        let mut cumulative = 0u64;
        let buckets: Vec<HistogramBucketValue> = m.histogram.iter().map(|&(le, count)| {
            cumulative += count;
            HistogramBucketValue {
                upper_bound: if le == u64::MAX { f64::INFINITY } else { le as f64 / 1_000_000.0 },
                cumulative_count: cumulative,
            }
        }).collect();

        dur_points.push(MetricPoint {
            labels,
            gauge_value: None,
            sum_value: Some(m.avg_duration_ms()),
            histogram_value: Some(HistogramValue {
                sample_count: m.calls,
                sample_sum: m.duration_sum_ns as f64 / 1_000_000.0,
                buckets,
            }),
            timestamp: now.clone(),
        });
    }

    MetricsResponse {
        metrics: vec![
            MetricFamily { name: "calls".into(), r#type: "GAUGE".into(), help: "Request rate per second".into(), metric_points: call_points },
            MetricFamily { name: "errors".into(), r#type: "GAUGE".into(), help: "Error rate per second".into(), metric_points: err_points },
            MetricFamily { name: "duration".into(), r#type: "HISTOGRAM".into(), help: "Span duration in ms".into(), metric_points: dur_points },
        ],
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::collections::HashMap;

    fn span(svc: &str, op: &str, dur_ns: u64, error: bool) -> Span {
        Span {
            trace_id: 1, span_id: 1, parent_span_id: None,
            operation_name: op.into(), service_name: svc.into(),
            start_time_unix_nano: 0, end_time_unix_nano: dur_ns, duration_ns: dur_ns,
            status: if error { SpanStatus::Error } else { SpanStatus::Ok },
            kind: SpanKind::Server,
            tags: HashMap::new(), events: vec![], links: vec![],
            resource_attributes: HashMap::new(),
            tenant_id: "default".into(), baggage: HashMap::new(), log_labels: HashMap::new(),
        }
    }

    #[test]
    fn spm_accumulates_calls() {
        let reg = SpmRegistry::new(60);
        reg.record_spans(&[span("svc", "op", 1_000_000, false), span("svc", "op", 2_000_000, true)]);
        let snap = reg.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].calls, 2);
        assert_eq!(snap[0].errors, 1);
    }

    #[test]
    fn spm_request_rate() {
        let reg = SpmRegistry::new(60);
        reg.record_spans(&vec![span("svc", "op", 1_000_000, false); 120]);
        let snap = reg.snapshot();
        assert!((snap[0].request_rate() - 2.0).abs() < 0.01);
    }

    #[test]
    fn to_prometheus_output() {
        let reg = SpmRegistry::new(60);
        reg.record_spans(&[span("api", "get", 5_000_000, false)]);
        let snap = reg.snapshot();
        let prom = to_prometheus(&snap);
        assert!(prom.contains("traces_spanmetrics_calls_total"));
        assert!(prom.contains("api"));
    }
}
