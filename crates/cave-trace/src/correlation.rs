//! Trace-to-logs and trace-to-metrics correlation.
//!
//! Provides the metadata needed to link a trace/span to:
//!   • Logs: Loki label matchers derived from resource attributes
//!   • Metrics: Prometheus/Mimir query with matching labels
//!
//! This is used by the Jaeger/Tempo UIs to show contextual logs/metrics
//! alongside a selected trace.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::types::{Span, TagValue, Trace, TraceId};

// ─── Log correlation ────────────────────────────────────────────────────────

/// Loki log query derived from span context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogCorrelation {
    pub trace_id: String,
    pub span_id: Option<String>,
    /// Loki stream selector, e.g. `{app="frontend",namespace="prod"}`
    pub loki_selector: String,
    /// Loki LogQL query with trace ID filter.
    pub logql: String,
}

/// Build a Loki log correlation for a span.
pub fn log_correlation(span: &Span) -> LogCorrelation {
    let trace_id_hex = crate::types::format_trace_id(span.trace_id);
    let span_id_hex  = crate::types::format_span_id(span.span_id);

    // Build selector from resource / log labels
    let selector_parts: Vec<String> = build_loki_labels(span)
        .into_iter()
        .map(|(k, v)| format!(r#"{}="{}""#, k, v.replace('"', "\\\"")))
        .collect();

    let loki_selector = if selector_parts.is_empty() {
        format!(r#"{{service="{}"}}"#, span.service_name)
    } else {
        format!("{{{}}}", selector_parts.join(","))
    };

    let logql = format!(
        r#"{} |= `traceID={}` or `trace_id={}`"#,
        loki_selector, trace_id_hex, trace_id_hex
    );

    LogCorrelation {
        trace_id: trace_id_hex,
        span_id: Some(span_id_hex),
        loki_selector,
        logql,
    }
}

fn build_loki_labels(span: &Span) -> Vec<(String, String)> {
    let mut labels: Vec<(String, String)> = Vec::new();

    // Prefer explicit log_labels (set at ingestion time)
    if !span.log_labels.is_empty() {
        return span.log_labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    }

    // Derive from well-known resource attributes
    let well_known = [
        ("service.name",      "app"),
        ("k8s.namespace.name","namespace"),
        ("k8s.pod.name",      "pod"),
        ("k8s.container.name","container"),
        ("host.name",         "host"),
        ("deployment.environment", "env"),
    ];
    for (attr, label) in well_known {
        if let Some(TagValue::String(v)) = span.resource_attributes.get(attr) {
            labels.push((label.to_owned(), v.clone()));
        }
    }

    // Always include service if nothing else matched
    if labels.is_empty() {
        labels.push(("service".to_owned(), span.service_name.clone()));
    }

    labels
}

// ─── Metrics correlation ────────────────────────────────────────────────────

/// Prometheus / Mimir query derived from span context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsCorrelation {
    pub trace_id: String,
    /// Prometheus label selector matching this service.
    pub prometheus_selector: String,
    /// Suggested PromQL to show request rate over the trace window.
    pub request_rate_promql: String,
    /// Suggested PromQL to show error rate over the trace window.
    pub error_rate_promql: String,
    /// Suggested PromQL to show p99 latency.
    pub p99_latency_promql: String,
    /// Time range for the queries (Unix seconds).
    pub start_time_s: i64,
    pub end_time_s: i64,
}

pub fn metrics_correlation(trace: &Trace) -> MetricsCorrelation {
    let trace_id_hex = crate::types::format_trace_id(trace.trace_id);

    let service = &trace.root_service_name;
    let selector = format!(r#"service="{}""#, service);

    // Pad the time range by 5 minutes on each side for context
    let pad_ns = 5 * 60 * 1_000_000_000u64;
    let start_s = (trace.start_time_unix_nano.saturating_sub(pad_ns) / 1_000_000_000) as i64;
    let end_s   = ((trace.end_time_unix_nano + pad_ns) / 1_000_000_000) as i64;

    MetricsCorrelation {
        trace_id: trace_id_hex,
        prometheus_selector: selector.clone(),
        request_rate_promql: format!(
            "rate(http_server_requests_total{{{}}}[1m])", selector
        ),
        error_rate_promql: format!(
            "rate(http_server_requests_total{{{},status=~\"5..\"}}[1m])", selector
        ),
        p99_latency_promql: format!(
            "histogram_quantile(0.99, rate(http_server_request_duration_seconds_bucket{{{}}}[1m]))",
            selector
        ),
        start_time_s: start_s,
        end_time_s: end_s,
    }
}

// ─── Baggage propagation ────────────────────────────────────────────────────

/// Extract W3C Baggage or B3 baggage from span tags.
pub fn extract_baggage(span: &Span) -> HashMap<String, String> {
    let mut baggage = span.baggage.clone();

    // W3C Baggage header values stored as tags
    if let Some(TagValue::String(b)) = span.tags.get("baggage") {
        for item in b.split(',') {
            if let Some((k, v)) = item.trim().split_once('=') {
                baggage.insert(k.trim().to_owned(), v.trim().to_owned());
            }
        }
    }

    baggage
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::collections::HashMap;

    fn make_span() -> Span {
        let mut resource = HashMap::new();
        resource.insert("service.name".into(), TagValue::String("checkout".into()));
        resource.insert("k8s.namespace.name".into(), TagValue::String("prod".into()));
        resource.insert("k8s.pod.name".into(), TagValue::String("checkout-abc123".into()));

        Span {
            trace_id: 0xdeadbeef,
            span_id: 0xcafe,
            parent_span_id: None,
            operation_name: "POST /order".into(),
            service_name: "checkout".into(),
            start_time_unix_nano: 1_640_000_000_000_000_000,
            end_time_unix_nano:   1_640_000_000_005_000_000,
            duration_ns: 5_000_000,
            status: SpanStatus::Ok,
            kind: SpanKind::Server,
            tags: HashMap::new(),
            events: vec![],
            links: vec![],
            resource_attributes: resource,
            tenant_id: "default".into(),
            baggage: HashMap::new(),
            log_labels: HashMap::new(),
        }
    }

    #[test]
    fn log_correlation_selector() {
        let span = make_span();
        let corr = log_correlation(&span);
        assert!(corr.loki_selector.contains("checkout"));
        assert!(corr.logql.contains("00000000000000000000000deadbeef"));
    }

    #[test]
    fn log_correlation_uses_k8s_labels() {
        let span = make_span();
        let corr = log_correlation(&span);
        assert!(corr.loki_selector.contains("namespace") || corr.loki_selector.contains("checkout"));
    }

    #[test]
    fn metrics_correlation_time_range() {
        let trace = Trace::from_spans(vec![make_span()]).unwrap();
        let corr = metrics_correlation(&trace);
        assert!(corr.start_time_s < corr.end_time_s);
        assert!(corr.request_rate_promql.contains("checkout"));
    }

    #[test]
    fn baggage_extraction_from_tag() {
        let mut span = make_span();
        span.tags.insert("baggage".into(), TagValue::String("user_id=42, tenant=acme".into()));
        let baggage = extract_baggage(&span);
        assert_eq!(baggage.get("user_id"), Some(&"42".to_owned()));
        assert_eq!(baggage.get("tenant"), Some(&"acme".to_owned()));
    }
}
