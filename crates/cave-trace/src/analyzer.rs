use crate::models::{
    Bottleneck, ErrorPropagation, ServiceDependency, ServiceMap, ServiceNode, Span, SpanStatus,
    TraceStats,
};
use std::collections::HashMap;
use uuid::Uuid;

// ── Service dependency map ────────────────────────────────────────────────────

/// Build a graph of which services call which other services, including
/// call counts, error counts, and average call duration.
pub fn service_dependency_map(spans: &[Span]) -> ServiceMap {
    let span_by_id: HashMap<Uuid, &Span> = spans.iter().map(|s| (s.span_id, s)).collect();

    // (name) -> (span_count, error_count, total_duration_ms)
    let mut svc_stats: HashMap<String, (u64, u64, f64)> = HashMap::new();
    // (source, target) -> (call_count, error_count, total_duration_ms)
    let mut dep_stats: HashMap<(String, String), (u64, u64, f64)> = HashMap::new();

    for span in spans {
        let e = svc_stats.entry(span.service.clone()).or_insert((0, 0, 0.0));
        e.0 += 1;
        if span.status == SpanStatus::Error {
            e.1 += 1;
        }
        e.2 += span.duration_ms;

        if let Some(pid) = span.parent_span_id {
            if let Some(parent) = span_by_id.get(&pid) {
                if parent.service != span.service {
                    let d = dep_stats
                        .entry((parent.service.clone(), span.service.clone()))
                        .or_insert((0, 0, 0.0));
                    d.0 += 1;
                    if span.status == SpanStatus::Error {
                        d.1 += 1;
                    }
                    d.2 += span.duration_ms;
                }
            }
        }
    }

    let services = svc_stats
        .into_iter()
        .map(|(name, (count, errors, total))| ServiceNode {
            name,
            span_count: count,
            error_count: errors,
            avg_duration_ms: if count > 0 {
                total / count as f64
            } else {
                0.0
            },
        })
        .collect();

    let dependencies = dep_stats
        .into_iter()
        .map(|((source, target), (count, errors, total))| ServiceDependency {
            source,
            target,
            call_count: count,
            error_count: errors,
            avg_duration_ms: if count > 0 {
                total / count as f64
            } else {
                0.0
            },
        })
        .collect();

    ServiceMap {
        services,
        dependencies,
    }
}

// ── Latency breakdown ─────────────────────────────────────────────────────────

/// Compute p50/p95/p99/avg latency and error rate per (service, operation) pair.
pub fn latency_breakdown(spans: &[Span]) -> Vec<TraceStats> {
    let mut groups: HashMap<(String, String), Vec<f64>> = HashMap::new();

    for span in spans {
        groups
            .entry((span.service.clone(), span.operation.clone()))
            .or_default()
            .push(span.duration_ms);
    }

    groups
        .into_iter()
        .map(|((service, operation), mut durations)| {
            durations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let count = durations.len();
            let avg = durations.iter().sum::<f64>() / count as f64;
            let errors = spans
                .iter()
                .filter(|s| {
                    s.service == service
                        && s.operation == operation
                        && s.status == SpanStatus::Error
                })
                .count();

            TraceStats {
                service,
                operation: Some(operation),
                total_spans: count as u64,
                error_rate: errors as f64 / count as f64,
                p50_ms: percentile(&durations, 50.0),
                p95_ms: percentile(&durations, 95.0),
                p99_ms: percentile(&durations, 99.0),
                avg_ms: avg,
            }
        })
        .collect()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

// ── Error propagation ─────────────────────────────────────────────────────────

/// For each trace that contains at least one error span, return the chain of
/// erroring span IDs, the inferred root-cause service, and all affected services.
pub fn error_propagation_analysis(spans: &[Span]) -> Vec<ErrorPropagation> {
    let mut by_trace: HashMap<Uuid, Vec<&Span>> = HashMap::new();
    for span in spans {
        by_trace.entry(span.trace_id).or_default().push(span);
    }

    by_trace
        .into_iter()
        .filter_map(|(trace_id, trace_spans)| {
            let error_spans: Vec<&Span> = trace_spans
                .iter()
                .copied()
                .filter(|s| s.status == SpanStatus::Error)
                .collect();

            if error_spans.is_empty() {
                return None;
            }

            let error_chain: Vec<Uuid> = error_spans.iter().map(|s| s.span_id).collect();
            let mut affected_services: Vec<String> =
                error_spans.iter().map(|s| s.service.clone()).collect();
            affected_services.sort();
            affected_services.dedup();

            // Treat the earliest-starting error span's service as root cause.
            let root_cause_service = error_spans
                .iter()
                .min_by_key(|s| s.start_time)
                .map(|s| s.service.clone())
                .unwrap_or_default();

            Some(ErrorPropagation {
                trace_id,
                error_chain,
                root_cause_service,
                affected_services,
                total_error_spans: error_spans.len(),
            })
        })
        .collect()
}

// ── Bottleneck detection ──────────────────────────────────────────────────────

/// Rank (service, operation) pairs by the total wall-clock time they consume
/// across all sampled spans, descending.
pub fn bottleneck_detection(spans: &[Span]) -> Vec<Bottleneck> {
    let total_time: f64 = spans.iter().map(|s| s.duration_ms).sum();

    let mut groups: HashMap<(String, String), (u64, f64)> = HashMap::new();
    for span in spans {
        let e = groups
            .entry((span.service.clone(), span.operation.clone()))
            .or_insert((0, 0.0));
        e.0 += 1;
        e.1 += span.duration_ms;
    }

    let mut bottlenecks: Vec<Bottleneck> = groups
        .into_iter()
        .map(|((service, operation), (count, total))| Bottleneck {
            service,
            operation,
            avg_duration_ms: if count > 0 {
                total / count as f64
            } else {
                0.0
            },
            call_count: count,
            total_time_ms: total,
            percentage_of_trace: if total_time > 0.0 {
                total / total_time * 100.0
            } else {
                0.0
            },
        })
        .collect();

    bottlenecks.sort_by(|a, b| {
        b.total_time_ms
            .partial_cmp(&a.total_time_ms)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    bottlenecks
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn span(service: &str, op: &str, dur: f64, status: SpanStatus) -> Span {
        Span {
            trace_id: Uuid::new_v4(),
            span_id: Uuid::new_v4(),
            parent_span_id: None,
            operation: op.into(),
            service: service.into(),
            start_time: Utc::now(),
            duration_ms: dur,
            status,
            tags: HashMap::new(),
            events: Vec::new(),
            links: Vec::new(),
        }
    }

    #[test]
    fn latency_breakdown_percentiles() {
        let spans = vec![
            span("api", "GET /", 10.0, SpanStatus::Ok),
            span("api", "GET /", 20.0, SpanStatus::Ok),
            span("api", "GET /", 100.0, SpanStatus::Error),
        ];
        let stats = latency_breakdown(&spans);
        assert_eq!(stats.len(), 1);
        assert!((stats[0].error_rate - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn bottleneck_orders_by_total_time() {
        let spans = vec![
            span("slow-svc", "query", 200.0, SpanStatus::Ok),
            span("fast-svc", "ping", 10.0, SpanStatus::Ok),
            span("fast-svc", "ping", 10.0, SpanStatus::Ok),
        ];
        let b = bottleneck_detection(&spans);
        assert_eq!(b[0].service, "slow-svc");
    }

    #[test]
    fn error_propagation_groups_by_trace() {
        let tid = Uuid::new_v4();
        let mut s1 = span("auth", "verify", 5.0, SpanStatus::Error);
        s1.trace_id = tid;
        let mut s2 = span("api", "login", 50.0, SpanStatus::Error);
        s2.trace_id = tid;
        let result = error_propagation_analysis(&[s1, s2]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_error_spans, 2);
    }
}
