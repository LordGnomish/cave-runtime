// SPDX-License-Identifier: AGPL-3.0-or-later
//! Advanced trace analytics.
//!
//! • Latency breakdown per (service, operation) with percentiles
//! • Error rate analysis with root-cause attribution
//! • Bottleneck detection (operations consuming most cumulative wall time)
//! • Service error propagation chains
//! • Anomaly detection (z-score on span duration)

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::storage::TraceStore;
use crate::types::{
    build_histogram, LatencyHistogram, Span, SpanStatus, TagValue, Trace, TraceId,
    TraceSearchQuery,
};

// ─── Latency breakdown ─────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LatencyBreakdown {
    pub service: String,
    pub operation: String,
    pub call_count: u64,
    pub error_count: u64,
    pub error_rate: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
}

pub fn latency_breakdown(spans: &[Span]) -> Vec<LatencyBreakdown> {
    let mut groups: HashMap<(String, String), Vec<u64>> = HashMap::new();
    let mut errors: HashMap<(String, String), u64> = HashMap::new();

    for span in spans {
        let key = (span.service_name.clone(), span.operation_name.clone());
        groups.entry(key.clone()).or_default().push(span.duration_ns);
        if span.has_error() {
            *errors.entry(key).or_insert(0) += 1;
        }
    }

    groups
        .into_iter()
        .map(|(key, mut durations)| {
            durations.sort_unstable();
            let count = durations.len() as u64;
            let err_count = *errors.get(&key).unwrap_or(&0);
            let sum: u64 = durations.iter().sum();
            let mean_ns = if count > 0 { sum / count } else { 0 };

            let pct = |p: f64| -> f64 {
                if durations.is_empty() { return 0.0; }
                let idx = ((p / 100.0) * (durations.len() - 1) as f64).round() as usize;
                durations[idx.min(durations.len() - 1)] as f64 / 1_000_000.0
            };

            LatencyBreakdown {
                service: key.0,
                operation: key.1,
                call_count: count,
                error_count: err_count,
                error_rate: if count > 0 { err_count as f64 / count as f64 } else { 0.0 },
                p50_ms: pct(50.0),
                p95_ms: pct(95.0),
                p99_ms: pct(99.0),
                min_ms: durations.first().copied().unwrap_or(0) as f64 / 1_000_000.0,
                max_ms: durations.last().copied().unwrap_or(0) as f64 / 1_000_000.0,
                mean_ms: mean_ns as f64 / 1_000_000.0,
            }
        })
        .collect()
}

// ─── Bottleneck detection ──────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Bottleneck {
    pub service: String,
    pub operation: String,
    /// Total nanoseconds this operation accounted for across all observed spans.
    pub total_duration_ns: u64,
    /// Number of times this operation appeared.
    pub call_count: u64,
    /// Average duration in ms.
    pub avg_ms: f64,
    /// Fraction of all measured time consumed by this operation.
    pub fraction_of_total: f64,
}

pub fn detect_bottlenecks(spans: &[Span]) -> Vec<Bottleneck> {
    let mut totals: HashMap<(String, String), (u64, u64)> = HashMap::new();
    let mut grand_total = 0u64;

    for span in spans {
        let key = (span.service_name.clone(), span.operation_name.clone());
        let e = totals.entry(key).or_insert((0, 0));
        e.0 += span.duration_ns;
        e.1 += 1;
        grand_total += span.duration_ns;
    }

    let mut list: Vec<Bottleneck> = totals
        .into_iter()
        .map(|(key, (total_ns, count))| Bottleneck {
            service: key.0,
            operation: key.1,
            total_duration_ns: total_ns,
            call_count: count,
            avg_ms: if count > 0 { total_ns as f64 / count as f64 / 1_000_000.0 } else { 0.0 },
            fraction_of_total: if grand_total > 0 { total_ns as f64 / grand_total as f64 } else { 0.0 },
        })
        .collect();

    list.sort_by(|a, b| b.total_duration_ns.cmp(&a.total_duration_ns));
    list
}

// ─── Error propagation analysis ────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ErrorChain {
    pub root_cause_service: String,
    pub root_cause_operation: String,
    pub propagated_to: Vec<String>,
    pub affected_trace_count: usize,
}

/// Identify which service / operation is the origin of errors that propagate
/// to callers. Looks at parent-child relationships within each trace.
pub fn error_propagation_analysis(traces: &[Trace]) -> Vec<ErrorChain> {
    use std::collections::HashSet;

    let mut root_causes: HashMap<(String, String), (HashSet<String>, usize)> = HashMap::new();

    for trace in traces {
        let span_map: HashMap<u64, &Span> =
            trace.spans.iter().map(|s| (s.span_id, s)).collect();

        // Find error spans that have no error parent → root cause
        for span in &trace.spans {
            if !span.has_error() { continue; }

            let parent_is_error = span.parent_span_id
                .and_then(|pid| span_map.get(&pid))
                .map(|p| p.has_error())
                .unwrap_or(false);

            if !parent_is_error {
                // This is an error origin
                let key = (span.service_name.clone(), span.operation_name.clone());
                let entry = root_causes.entry(key).or_insert((HashSet::new(), 0));
                entry.1 += 1;

                // Collect all services that have error spans with this as ancestor
                for other in &trace.spans {
                    if other.has_error() && other.span_id != span.span_id {
                        entry.0.insert(other.service_name.clone());
                    }
                }
            }
        }
    }

    root_causes
        .into_iter()
        .map(|((svc, op), (propagated, count))| ErrorChain {
            root_cause_service: svc,
            root_cause_operation: op,
            propagated_to: propagated.into_iter().collect(),
            affected_trace_count: count,
        })
        .collect()
}

// ─── Anomaly detection (z-score) ──────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AnomalousSpan {
    pub trace_id: String,
    pub span_id: String,
    pub service: String,
    pub operation: String,
    pub duration_ms: f64,
    pub z_score: f64,
    pub baseline_mean_ms: f64,
    pub baseline_std_ms: f64,
}

/// Flag spans whose duration is more than `threshold_sigma` standard deviations
/// above the mean for their (service, operation) group.
pub fn detect_anomalous_spans(spans: &[Span], threshold_sigma: f64) -> Vec<AnomalousSpan> {
    let mut groups: HashMap<(String, String), Vec<f64>> = HashMap::new();
    for span in spans {
        let key = (span.service_name.clone(), span.operation_name.clone());
        groups.entry(key).or_default().push(span.duration_ns as f64 / 1_000_000.0);
    }

    // Compute mean + std per group
    let stats: HashMap<(String, String), (f64, f64)> = groups
        .iter()
        .map(|(k, vals)| {
            let n = vals.len() as f64;
            let mean = vals.iter().sum::<f64>() / n;
            let variance = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
            (k.clone(), (mean, variance.sqrt()))
        })
        .collect();

    let mut anomalies = Vec::new();
    for span in spans {
        let key = (span.service_name.clone(), span.operation_name.clone());
        if let Some(&(mean, std)) = stats.get(&key) {
            if std < 0.001 { continue; } // no variance, skip
            let dur_ms = span.duration_ns as f64 / 1_000_000.0;
            let z = (dur_ms - mean) / std;
            if z > threshold_sigma {
                anomalies.push(AnomalousSpan {
                    trace_id: crate::types::format_trace_id(span.trace_id),
                    span_id: crate::types::format_span_id(span.span_id),
                    service: span.service_name.clone(),
                    operation: span.operation_name.clone(),
                    duration_ms: dur_ms,
                    z_score: z,
                    baseline_mean_ms: mean,
                    baseline_std_ms: std,
                });
            }
        }
    }

    anomalies.sort_by(|a, b| b.z_score.partial_cmp(&a.z_score).unwrap_or(std::cmp::Ordering::Equal));
    anomalies
}

// ─── Service performance metrics (aggregated) ─────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceMetrics {
    pub service: String,
    pub request_rate: f64,   // req/s
    pub error_rate: f64,     // errors/s
    pub error_fraction: f64, // 0..1
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub total_spans: u64,
}

/// Aggregate per-service RED (Rate / Errors / Duration) metrics.
pub fn service_metrics(spans: &[Span], window_secs: f64) -> Vec<ServiceMetrics> {
    let mut groups: HashMap<String, Vec<u64>> = HashMap::new();
    let mut errs: HashMap<String, u64> = HashMap::new();

    for span in spans {
        groups.entry(span.service_name.clone()).or_default().push(span.duration_ns);
        if span.has_error() {
            *errs.entry(span.service_name.clone()).or_insert(0) += 1;
        }
    }

    groups
        .into_iter()
        .map(|(svc, mut durs)| {
            durs.sort_unstable();
            let count = durs.len() as u64;
            let err_count = *errs.get(&svc).unwrap_or(&0);

            let pct = |p: f64| -> f64 {
                if durs.is_empty() { return 0.0; }
                let idx = ((p / 100.0) * (durs.len() - 1) as f64).round() as usize;
                durs[idx.min(durs.len() - 1)] as f64 / 1_000_000.0
            };

            ServiceMetrics {
                service: svc,
                request_rate: if window_secs > 0.0 { count as f64 / window_secs } else { 0.0 },
                error_rate: if window_secs > 0.0 { err_count as f64 / window_secs } else { 0.0 },
                error_fraction: if count > 0 { err_count as f64 / count as f64 } else { 0.0 },
                p50_ms: pct(50.0),
                p95_ms: pct(95.0),
                p99_ms: pct(99.0),
                total_spans: count,
            }
        })
        .collect()
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::collections::HashMap;

    fn span(svc: &str, op: &str, dur_ns: u64, error: bool) -> Span {
        Span {
            trace_id: 1,
            span_id: 1,
            parent_span_id: None,
            operation_name: op.into(),
            service_name: svc.into(),
            start_time_unix_nano: 0,
            end_time_unix_nano: dur_ns,
            duration_ns: dur_ns,
            status: if error { SpanStatus::Error } else { SpanStatus::Ok },
            kind: SpanKind::Server,
            tags: HashMap::new(),
            events: vec![],
            links: vec![],
            resource_attributes: HashMap::new(),
            tenant_id: "default".into(),
            baggage: HashMap::new(),
            log_labels: HashMap::new(),
        }
    }

    #[test]
    fn latency_breakdown_percentiles() {
        let spans = vec![
            span("svc", "op", 1_000_000, false),   // 1 ms
            span("svc", "op", 10_000_000, false),  // 10 ms
            span("svc", "op", 100_000_000, false), // 100 ms
        ];
        let stats = latency_breakdown(&spans);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].call_count, 3);
        assert!(stats[0].p50_ms > 0.0);
        assert!(stats[0].p99_ms >= stats[0].p50_ms);
    }

    #[test]
    fn latency_breakdown_error_rate() {
        let spans = vec![
            span("svc", "op", 1_000_000, false),
            span("svc", "op", 1_000_000, true),
        ];
        let stats = latency_breakdown(&spans);
        assert!((stats[0].error_rate - 0.5).abs() < 0.01);
    }

    #[test]
    fn bottleneck_ranks_by_total_time() {
        let spans = vec![
            span("svc", "slow", 100_000_000, false),
            span("svc", "slow", 100_000_000, false),
            span("svc", "fast", 1_000_000, false),
        ];
        let bns = detect_bottlenecks(&spans);
        assert_eq!(bns[0].operation, "slow");
    }

    #[test]
    fn anomaly_detection_flags_outlier() {
        // 9 spans at 1ms, 1 at 100ms
        let mut spans: Vec<Span> = (0..9)
            .map(|_| span("svc", "op", 1_000_000, false))
            .collect();
        spans.push(span("svc", "op", 100_000_000, false));

        let anomalies = detect_anomalous_spans(&spans, 2.0);
        assert!(!anomalies.is_empty());
        assert!(anomalies[0].duration_ms > 50.0);
    }

    #[test]
    fn service_metrics_red() {
        let spans = vec![
            span("api", "get", 5_000_000, false),
            span("api", "get", 5_000_000, false),
            span("api", "get", 5_000_000, true),
        ];
        let metrics = service_metrics(&spans, 1.0);
        let api = metrics.iter().find(|m| m.service == "api").unwrap();
        assert_eq!(api.total_spans, 3);
        assert!((api.error_fraction - 1.0/3.0).abs() < 0.01);
    }
}
