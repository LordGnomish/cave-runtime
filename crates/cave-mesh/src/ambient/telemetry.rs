//! Mesh telemetry — access log + Prometheus metric + OpenTelemetry trace span.
//!
//! Mirrors `pilot/pkg/networking/telemetry/telemetry.go` plus the access-log
//! formatter in `pkg/util/log/access.go`. The three sinks share one
//! `RequestRecord` source-of-truth so a single waypoint dispatch can feed all
//! observability layers.

use crate::ambient::types::{Cite, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One end-to-end record of a request as it leaves the waypoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestRecord {
    pub tenant: TenantId,
    pub timestamp: DateTime<Utc>,
    pub source_principal: String,
    pub destination: String,
    pub method: String,
    pub path: String,
    pub response_code: u16,
    pub duration_ms: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    /// W3C trace context — present iff the upstream sent `traceparent`.
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

/// Access-log formatter — Istio's standard format with the tenant prepended.
///
/// Format:
/// ```text
/// [<rfc3339>] tenant=<t> "<METHOD> <path>" <code> <dur_ms>ms src=<principal> dst=<dest> bytes=<sent>/<recv>
/// ```
pub fn format_access_log(r: &RequestRecord) -> String {
    format!(
        "[{}] tenant={} \"{} {}\" {} {}ms src={} dst={} bytes={}/{}",
        r.timestamp.to_rfc3339(),
        r.tenant,
        r.method,
        r.path,
        r.response_code,
        r.duration_ms,
        r.source_principal,
        r.destination,
        r.bytes_sent,
        r.bytes_received,
    )
}

/// In-memory Prometheus-shaped counter store.
///
/// Mirrors the Istio metric set defined in `pkg/monitoring/`:
/// `istio_requests_total`, `istio_request_duration_milliseconds_sum`, etc.
#[derive(Debug, Default, Clone)]
pub struct PromRegistry {
    /// Map of (metric_name, label-set) → counter value. Label-set is a
    /// sorted `BTreeMap` for stable hashing.
    pub counters: std::collections::HashMap<(String, BTreeMap<String, String>), u64>,
}

/// Standard label keys.
pub const LABEL_TENANT: &str = "tenant";
pub const LABEL_METHOD: &str = "method";
pub const LABEL_RESPONSE_CODE: &str = "response_code";
pub const LABEL_DESTINATION: &str = "destination_workload";
pub const LABEL_SOURCE: &str = "source_principal";

pub const METRIC_REQUESTS_TOTAL: &str = "istio_requests_total";
pub const METRIC_REQUEST_DURATION_SUM: &str = "istio_request_duration_milliseconds_sum";
pub const METRIC_BYTES_SENT_SUM: &str = "istio_response_bytes_sum";

impl PromRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn labels_for(r: &RequestRecord) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert(LABEL_TENANT.into(), r.tenant.to_string());
        m.insert(LABEL_METHOD.into(), r.method.clone());
        m.insert(LABEL_RESPONSE_CODE.into(), r.response_code.to_string());
        m.insert(LABEL_DESTINATION.into(), r.destination.clone());
        m.insert(LABEL_SOURCE.into(), r.source_principal.clone());
        m
    }

    /// Emit the standard Istio metric set for a single request.
    pub fn observe(&mut self, r: &RequestRecord) {
        let labels = Self::labels_for(r);
        *self.counters.entry((METRIC_REQUESTS_TOTAL.into(), labels.clone())).or_insert(0) += 1;
        *self.counters.entry((METRIC_REQUEST_DURATION_SUM.into(), labels.clone())).or_insert(0) +=
            r.duration_ms;
        *self.counters.entry((METRIC_BYTES_SENT_SUM.into(), labels)).or_insert(0) += r.bytes_sent;
    }

    /// Lookup helper used by tests; returns 0 if the cell hasn't been written.
    pub fn get(&self, metric: &str, labels: &BTreeMap<String, String>) -> u64 {
        *self.counters.get(&(metric.to_string(), labels.clone())).unwrap_or(&0)
    }
}

/// OpenTelemetry span. Real export happens through cave's OTLP pipeline; here
/// we model the fields the Ambient stack writes so callers can assert on them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtelSpan {
    pub trace_id: String,
    pub span_id: String,
    pub name: String,
    pub start: DateTime<Utc>,
    pub duration_ms: u64,
    pub attributes: Vec<(String, String)>,
    pub status: SpanStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanStatus {
    Ok,
    Error { http_code: u16 },
}

/// Build an OpenTelemetry span from a `RequestRecord`. Returns `None` when no
/// trace context is present (Istio never synthesises a span without an
/// upstream `traceparent`, except when the proxy is the trace root — out of
/// scope here).
pub fn build_span(r: &RequestRecord) -> Option<OtelSpan> {
    let trace_id = r.trace_id.clone()?;
    let span_id = r.span_id.clone()?;
    let attributes = vec![
        ("tenant".into(), r.tenant.to_string()),
        ("http.method".into(), r.method.clone()),
        ("http.target".into(), r.path.clone()),
        ("http.status_code".into(), r.response_code.to_string()),
        ("source.principal".into(), r.source_principal.clone()),
        ("destination.workload".into(), r.destination.clone()),
    ];
    let status = if r.response_code >= 500 {
        SpanStatus::Error { http_code: r.response_code }
    } else {
        SpanStatus::Ok
    };
    Some(OtelSpan {
        trace_id,
        span_id,
        name: format!("{} {}", r.method, r.path),
        start: r.timestamp,
        duration_ms: r.duration_ms,
        attributes,
        status,
    })
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::istio("pilot/pkg/networking/telemetry/telemetry.go", "telemetryFilters");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambient_test_ctx;

    fn rec(method: &str, code: u16, with_trace: bool) -> RequestRecord {
        RequestRecord {
            tenant: TenantId::new("acme"),
            timestamp: DateTime::parse_from_rfc3339("2026-04-26T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            source_principal: "spiffe://cluster.local/ns/acme/sa/web".into(),
            destination: "api.acme.svc".into(),
            method: method.into(),
            path: "/v1/users".into(),
            response_code: code,
            duration_ms: 42,
            bytes_sent: 1024,
            bytes_received: 256,
            trace_id: with_trace.then(|| "0af7651916cd43dd8448eb211c80319c".to_string()),
            span_id: with_trace.then(|| "b7ad6b7169203331".to_string()),
        }
    }

    #[test]
    fn access_log_includes_tenant_method_path_and_code() {
        let (_cite, _t) = ambient_test_ctx!(
            "pkg/util/log/access.go",
            "FormatAccessLog",
            "tenant-tel-access"
        );
        let line = format_access_log(&rec("POST", 201, false));
        assert!(line.contains("tenant=acme"));
        assert!(line.contains("\"POST /v1/users\""));
        assert!(line.contains(" 201 42ms"));
        assert!(line.contains("dst=api.acme.svc"));
        assert!(line.contains("bytes=1024/256"));
    }

    #[test]
    fn observe_increments_requests_total_with_full_label_set() {
        let (_cite, _t) = ambient_test_ctx!(
            "pkg/monitoring/monitoring.go",
            "Counter.Increment",
            "tenant-tel-prom"
        );
        let mut reg = PromRegistry::new();
        reg.observe(&rec("GET", 200, false));
        reg.observe(&rec("GET", 200, false));
        let labels = PromRegistry::labels_for(&rec("GET", 200, false));
        assert_eq!(reg.get(METRIC_REQUESTS_TOTAL, &labels), 2);
        assert_eq!(reg.get(METRIC_REQUEST_DURATION_SUM, &labels), 84);
        assert_eq!(reg.get(METRIC_BYTES_SENT_SUM, &labels), 2048);
    }

    #[test]
    fn observe_separates_metric_streams_by_response_code() {
        let (_cite, _t) = ambient_test_ctx!(
            "pkg/monitoring/monitoring.go",
            "WithLabels",
            "tenant-tel-prom-codes"
        );
        let mut reg = PromRegistry::new();
        reg.observe(&rec("GET", 200, false));
        reg.observe(&rec("GET", 500, false));
        let ok_labels = PromRegistry::labels_for(&rec("GET", 200, false));
        let err_labels = PromRegistry::labels_for(&rec("GET", 500, false));
        assert_eq!(reg.get(METRIC_REQUESTS_TOTAL, &ok_labels), 1);
        assert_eq!(reg.get(METRIC_REQUESTS_TOTAL, &err_labels), 1);
    }

    #[test]
    fn span_emitted_when_trace_context_is_present() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/telemetry/telemetry.go",
            "buildSpan",
            "tenant-tel-span"
        );
        let span = build_span(&rec("GET", 200, true)).unwrap();
        assert_eq!(span.trace_id, "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(span.name, "GET /v1/users");
        assert_eq!(span.status, SpanStatus::Ok);
        assert!(span
            .attributes
            .iter()
            .any(|(k, v)| k == "http.status_code" && v == "200"));
    }

    #[test]
    fn span_marks_5xx_response_as_error() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/telemetry/telemetry.go",
            "spanStatus",
            "tenant-tel-span-err"
        );
        let span = build_span(&rec("GET", 503, true)).unwrap();
        assert_eq!(span.status, SpanStatus::Error { http_code: 503 });
    }

    #[test]
    fn span_skipped_when_no_trace_context_present() {
        let (_cite, _t) = ambient_test_ctx!(
            "pilot/pkg/networking/telemetry/telemetry.go",
            "buildSpan",
            "tenant-tel-no-trace"
        );
        assert!(build_span(&rec("GET", 200, false)).is_none());
    }
}
