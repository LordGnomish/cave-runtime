// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core data model for cave-trace.
//!
//! TraceId is a 128-bit unsigned integer (wire format: lowercase 32-char hex or base64).
//! SpanId is a 64-bit unsigned integer (wire format: lowercase 16-char hex or base64).

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::{Result, TraceError};

// ─── Identifier types ──────────────────────────────────────────────────────

/// 128-bit trace identifier.
pub type TraceId = u128;
/// 64-bit span identifier.
pub type SpanId = u64;

/// Parse a trace ID from a lowercase hex string (up to 32 chars) or base64 (24 chars).
pub fn parse_trace_id(s: &str) -> Result<TraceId> {
    let s = s.trim();
    // If it looks like hex (1-32 hex chars), parse as hex
    if !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return u128::from_str_radix(s, 16)
            .map_err(|e| TraceError::InvalidTraceId(s.to_owned(), e.to_string()));
    }
    // Try base64 (OTLP JSON encodes bytes as base64)
    decode_base64_u128(s)
        .ok_or_else(|| TraceError::InvalidTraceId(s.to_owned(), "not hex or base64".into()))
}

/// Parse a span ID from a lowercase hex string (up to 16 chars) or base64 (12 chars).
pub fn parse_span_id(s: &str) -> Result<SpanId> {
    let s = s.trim();
    // If it looks like hex (1-16 hex chars), parse as hex
    if !s.is_empty() && s.len() <= 16 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return u64::from_str_radix(s, 16)
            .map_err(|e| TraceError::InvalidSpanId(s.to_owned(), e.to_string()));
    }
    decode_base64_u64(s)
        .ok_or_else(|| TraceError::InvalidSpanId(s.to_owned(), "not hex or base64".into()))
}

pub fn format_trace_id(id: TraceId) -> String {
    format!("{:032x}", id)
}

pub fn format_span_id(id: SpanId) -> String {
    format!("{:016x}", id)
}

fn decode_base64_u128(s: &str) -> Option<u128> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let bytes = STANDARD.decode(s).ok()?;
    if bytes.len() != 16 {
        return None;
    }
    Some(u128::from_be_bytes(bytes.try_into().ok()?))
}

fn decode_base64_u64(s: &str) -> Option<u64> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let bytes = STANDARD.decode(s).ok()?;
    if bytes.len() != 8 {
        return None;
    }
    Some(u64::from_be_bytes(bytes.try_into().ok()?))
}

// ─── Tag / attribute value ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TagValue {
    String(String),
    Bool(bool),
    Int(i64),
    Float(f64),
    Binary(Vec<u8>),
}

impl TagValue {
    pub fn as_str(&self) -> Option<&str> {
        if let TagValue::String(s) = self { Some(s) } else { None }
    }

    pub fn display(&self) -> String {
        match self {
            TagValue::String(s) => s.clone(),
            TagValue::Bool(b) => b.to_string(),
            TagValue::Int(i) => i.to_string(),
            TagValue::Float(f) => f.to_string(),
            TagValue::Binary(b) => b.iter().map(|x| format!("{:02x}", x)).collect(),
        }
    }

    pub fn matches_str(&self, pattern: &str) -> bool {
        self.display() == pattern
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            TagValue::Int(i) => Some(*i),
            TagValue::Float(f) => Some(*f as i64),
            TagValue::Bool(b) => Some(if *b { 1 } else { 0 }),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            TagValue::Float(f) => Some(*f),
            TagValue::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
}

// ─── Span status & kind ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SpanStatus {
    #[default]
    Unset,
    Ok,
    Error,
}

impl SpanStatus {
    pub fn is_error(self) -> bool {
        self == SpanStatus::Error
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SpanKind {
    #[default]
    Internal,
    Server,
    Client,
    Producer,
    Consumer,
}

// ─── Span event ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanEvent {
    pub time_unix_nano: u64,
    pub name: String,
    pub attributes: HashMap<String, TagValue>,
}

// ─── Span link ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanLink {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub trace_state: String,
    pub attributes: HashMap<String, TagValue>,
}

// ─── Process ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Process {
    pub service_name: String,
    pub tags: HashMap<String, TagValue>,
}

// ─── Span ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub parent_span_id: Option<SpanId>,
    pub operation_name: String,
    pub service_name: String,

    /// Epoch nanoseconds.
    pub start_time_unix_nano: u64,
    /// Epoch nanoseconds.
    pub end_time_unix_nano: u64,
    /// Derived: end - start in nanoseconds.
    pub duration_ns: u64,

    pub status: SpanStatus,
    pub kind: SpanKind,

    /// Span-level attributes / tags.
    pub tags: HashMap<String, TagValue>,
    pub events: Vec<SpanEvent>,
    pub links: Vec<SpanLink>,

    /// Resource attributes (from OTLP Resource or Jaeger Process).
    pub resource_attributes: HashMap<String, TagValue>,

    /// Multi-tenancy scope.
    pub tenant_id: String,

    /// Baggage items propagated with this span.
    pub baggage: HashMap<String, String>,

    /// Correlation: log stream selectors derived from resource attributes.
    pub log_labels: HashMap<String, String>,
}

impl Span {
    pub fn duration_ms(&self) -> f64 {
        self.duration_ns as f64 / 1_000_000.0
    }

    pub fn duration_sec(&self) -> f64 {
        self.duration_ns as f64 / 1_000_000_000.0
    }

    pub fn is_root(&self) -> bool {
        self.parent_span_id.is_none()
    }

    pub fn has_error(&self) -> bool {
        if self.status.is_error() {
            return true;
        }
        // Zipkin-style error tag
        if let Some(v) = self.tags.get("error") {
            return v.display() != "false";
        }
        // OpenTelemetry http.status_code
        if let Some(v) = self.tags.get("http.status_code") {
            if let Some(code) = v.as_i64() {
                return code >= 500;
            }
        }
        false
    }
}

// ─── Assembled Trace ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub trace_id: TraceId,
    pub root_service_name: String,
    pub root_operation_name: String,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    /// End-to-end latency in ns.
    pub duration_ns: u64,
    pub spans: Vec<Span>,
    pub span_count: usize,
    pub service_count: usize,
    pub error_count: usize,
    pub tenant_id: String,
}

impl Trace {
    /// Build a Trace from a set of spans (all must share the same trace_id).
    pub fn from_spans(spans: Vec<Span>) -> Option<Self> {
        if spans.is_empty() {
            return None;
        }

        let trace_id = spans[0].trace_id;
        let start = spans.iter().map(|s| s.start_time_unix_nano).min()?;
        let end = spans.iter().map(|s| s.end_time_unix_nano).max()?;

        let root = spans
            .iter()
            .find(|s| s.is_root())
            .or_else(|| spans.first())?;

        let services: std::collections::HashSet<&str> =
            spans.iter().map(|s| s.service_name.as_str()).collect();

        let error_count = spans.iter().filter(|s| s.has_error()).count();
        let tenant_id = root.tenant_id.clone();

        Some(Trace {
            trace_id,
            root_service_name: root.service_name.clone(),
            root_operation_name: root.operation_name.clone(),
            start_time_unix_nano: start,
            end_time_unix_nano: end,
            duration_ns: end.saturating_sub(start),
            span_count: spans.len(),
            service_count: services.len(),
            error_count,
            tenant_id,
            spans,
        })
    }
}

// ─── Span tree (for critical path / comparison) ───────────────────────────

#[derive(Debug, Clone)]
pub struct SpanNode {
    pub span: Span,
    pub children: Vec<SpanNode>,
}

impl SpanNode {
    /// Build a tree from a flat list. Returns (roots, orphans).
    pub fn build_forest(spans: &[Span]) -> Vec<SpanNode> {
        use std::collections::HashMap;
        let mut children_map: HashMap<SpanId, Vec<usize>> = HashMap::new();
        let mut root_indices: Vec<usize> = Vec::new();

        for (i, span) in spans.iter().enumerate() {
            match span.parent_span_id {
                Some(pid) => children_map.entry(pid).or_default().push(i),
                None => root_indices.push(i),
            }
        }

        // If no explicit root found, use the earliest span
        if root_indices.is_empty() {
            if let Some((idx, _)) = spans
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.start_time_unix_nano)
            {
                root_indices.push(idx);
            }
        }

        fn build_node(idx: usize, spans: &[Span], cm: &HashMap<SpanId, Vec<usize>>) -> SpanNode {
            let span = spans[idx].clone();
            let children = cm
                .get(&span.span_id)
                .map(|idxs| idxs.iter().map(|&i| build_node(i, spans, cm)).collect())
                .unwrap_or_default();
            SpanNode { span, children }
        }

        root_indices
            .iter()
            .map(|&i| build_node(i, spans, &children_map))
            .collect()
    }

    /// Duration in ns of this node's span.
    pub fn duration_ns(&self) -> u64 {
        self.span.duration_ns
    }

    /// Find the critical path: sequence of spans that determines end-to-end latency.
    /// Uses: critical_path_length = max over children of (child_start_offset + child_critical_path)
    pub fn critical_path(&self) -> Vec<SpanId> {
        if self.children.is_empty() {
            return vec![self.span.span_id];
        }

        // Find child whose critical sub-path ends latest
        let best = self.children.iter().max_by_key(|c| {
            let start_offset = c.span.start_time_unix_nano.saturating_sub(self.span.start_time_unix_nano);
            start_offset + c.critical_path_length()
        });

        let mut path = vec![self.span.span_id];
        if let Some(child) = best {
            path.extend(child.critical_path());
        }
        path
    }

    fn critical_path_length(&self) -> u64 {
        if self.children.is_empty() {
            return self.span.duration_ns;
        }
        let child_max = self.children.iter().map(|c| {
            let start_offset = c.span.start_time_unix_nano.saturating_sub(self.span.start_time_unix_nano);
            start_offset + c.critical_path_length()
        }).max().unwrap_or(0);
        self.span.duration_ns.max(child_max)
    }
}

// ─── Query / filter types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceSearchQuery {
    pub tenant_id: Option<String>,
    pub service: Option<String>,
    pub operation: Option<String>,
    pub tags: Option<HashMap<String, String>>,
    pub min_duration_ns: Option<u64>,
    pub max_duration_ns: Option<u64>,
    pub start_time_ns: Option<u64>,
    pub end_time_ns: Option<u64>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl TraceSearchQuery {
    pub fn limit_or_default(&self) -> usize {
        self.limit.unwrap_or(20).min(1000)
    }
}

// ─── Latency histogram ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyHistogram {
    pub service: String,
    pub operation: String,
    pub buckets: Vec<HistogramBucket>,
    pub count: u64,
    pub sum_ns: u64,
    pub p50_ns: u64,
    pub p75_ns: u64,
    pub p90_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub min_ns: u64,
    pub max_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramBucket {
    /// Upper bound in nanoseconds.
    pub le_ns: u64,
    pub count: u64,
}

/// Standard Prometheus-style latency buckets in nanoseconds.
pub const LATENCY_BUCKETS_NS: &[u64] = &[
    500_000,      // 0.5 ms
    1_000_000,    // 1 ms
    2_500_000,    // 2.5 ms
    5_000_000,    // 5 ms
    10_000_000,   // 10 ms
    25_000_000,   // 25 ms
    50_000_000,   // 50 ms
    100_000_000,  // 100 ms
    250_000_000,  // 250 ms
    500_000_000,  // 500 ms
    1_000_000_000, // 1 s
    2_500_000_000, // 2.5 s
    5_000_000_000, // 5 s
    u64::MAX,      // +Inf
];

/// Compute a latency histogram from a sorted slice of duration values.
pub fn build_histogram(
    service: String,
    operation: String,
    mut durations_ns: Vec<u64>,
) -> LatencyHistogram {
    if durations_ns.is_empty() {
        return LatencyHistogram {
            service,
            operation,
            buckets: LATENCY_BUCKETS_NS.iter().map(|&le| HistogramBucket { le_ns: le, count: 0 }).collect(),
            count: 0,
            sum_ns: 0,
            p50_ns: 0,
            p75_ns: 0,
            p90_ns: 0,
            p95_ns: 0,
            p99_ns: 0,
            min_ns: 0,
            max_ns: 0,
        };
    }

    durations_ns.sort_unstable();
    let count = durations_ns.len() as u64;
    let sum_ns: u64 = durations_ns.iter().sum();

    let percentile = |p: f64| -> u64 {
        let idx = ((p / 100.0) * (durations_ns.len() - 1) as f64).round() as usize;
        durations_ns[idx.min(durations_ns.len() - 1)]
    };

    let buckets = LATENCY_BUCKETS_NS
        .iter()
        .map(|&le| {
            let count = durations_ns.iter().filter(|&&d| d <= le).count() as u64;
            HistogramBucket { le_ns: le, count }
        })
        .collect();

    LatencyHistogram {
        service,
        operation,
        buckets,
        count,
        sum_ns,
        p50_ns: percentile(50.0),
        p75_ns: percentile(75.0),
        p90_ns: percentile(90.0),
        p95_ns: percentile(95.0),
        p99_ns: percentile(99.0),
        min_ns: *durations_ns.first().unwrap(),
        max_ns: *durations_ns.last().unwrap(),
    }
}

// ─── Service dependency edge ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ServiceEdge {
    pub parent: String,
    pub child: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDependency {
    pub parent: String,
    pub child: String,
    pub call_count: u64,
    pub error_count: u64,
    pub total_duration_ns: u64,
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_trace_id_hex() {
        let id: TraceId = 0xdeadbeefcafe1234_0011223344556677;
        let hex = format_trace_id(id);
        assert_eq!(hex.len(), 32);
        let parsed = parse_trace_id(&hex).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn roundtrip_span_id_hex() {
        let id: SpanId = 0xdeadbeefcafe1234;
        let hex = format_span_id(id);
        assert_eq!(hex.len(), 16);
        let parsed = parse_span_id(&hex).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn parse_base64_trace_id() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let bytes: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let b64 = STANDARD.encode(bytes);
        let id = parse_trace_id(&b64).unwrap();
        assert_eq!(id, u128::from_be_bytes(bytes));
    }

    #[test]
    fn tag_value_display() {
        assert_eq!(TagValue::Int(42).display(), "42");
        assert_eq!(TagValue::Bool(true).display(), "true");
        assert_eq!(TagValue::Float(3.14).display(), "3.14");
        assert_eq!(TagValue::String("hi".into()).display(), "hi");
    }

    #[test]
    fn span_has_error_by_status() {
        let mut s = make_span();
        s.status = SpanStatus::Error;
        assert!(s.has_error());
    }

    #[test]
    fn span_has_error_by_http_status() {
        let mut s = make_span();
        s.tags.insert("http.status_code".into(), TagValue::Int(503));
        assert!(s.has_error());
    }

    #[test]
    fn trace_from_spans_derives_duration() {
        let root = make_span();
        let trace = Trace::from_spans(vec![root]).unwrap();
        assert_eq!(trace.duration_ns, 1_000_000);
    }

    fn make_span() -> Span {
        Span {
            trace_id: 1,
            span_id: 1,
            parent_span_id: None,
            operation_name: "op".into(),
            service_name: "svc".into(),
            start_time_unix_nano: 1_000_000_000,
            end_time_unix_nano:   1_001_000_000,
            duration_ns: 1_000_000,
            status: SpanStatus::Ok,
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
}
