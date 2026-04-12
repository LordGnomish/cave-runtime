use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type TraceId = String;
pub type SpanId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub parent_span_id: Option<SpanId>,
    pub operation_name: String,
    pub service_name: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub duration_us: i64,
    pub status: SpanStatus,
    pub kind: SpanKind,
    pub tags: HashMap<String, AttributeValue>,
    pub events: Vec<SpanEvent>,
    pub links: Vec<SpanLink>,
    pub resource_attributes: HashMap<String, AttributeValue>,
}

impl Span {
    pub fn duration_ms(&self) -> f64 {
        self.duration_us as f64 / 1000.0
    }

    pub fn is_root(&self) -> bool {
        self.parent_span_id.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SpanStatus {
    Unset,
    Ok,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SpanKind {
    Unspecified,
    Internal,
    Server,
    Client,
    Producer,
    Consumer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttributeValue {
    String(String),
    Bool(bool),
    Int(i64),
    Double(f64),
    StringArray(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanEvent {
    pub name: String,
    pub timestamp: DateTime<Utc>,
    pub attributes: HashMap<String, AttributeValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanLink {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub attributes: HashMap<String, AttributeValue>,
}

/// A full trace = collection of spans sharing a trace_id
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub trace_id: TraceId,
    pub service_name: String,
    pub operation_name: String,
    pub start_time: DateTime<Utc>,
    pub duration_us: i64,
    pub span_count: usize,
    pub error_count: usize,
    pub spans: Vec<Span>,
}

impl Trace {
    pub fn from_spans(spans: Vec<Span>) -> Option<Trace> {
        if spans.is_empty() {
            return None;
        }
        let root = spans
            .iter()
            .find(|s| s.parent_span_id.is_none())
            .or_else(|| spans.first())?;
        let duration_us = spans.iter().map(|s| s.duration_us).max().unwrap_or(0);
        let error_count = spans
            .iter()
            .filter(|s| s.status == SpanStatus::Error)
            .count();
        Some(Trace {
            trace_id: root.trace_id.clone(),
            service_name: root.service_name.clone(),
            operation_name: root.operation_name.clone(),
            start_time: root.start_time,
            duration_us,
            span_count: spans.len(),
            error_count,
            spans,
        })
    }
}

/// Query parameters for finding traces
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TraceQuery {
    pub service_name: Option<String>,
    pub operation_name: Option<String>,
    pub tags: Option<HashMap<String, String>>,
    pub min_duration_us: Option<i64>,
    pub max_duration_us: Option<i64>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_span(trace_id: &str, span_id: &str, parent: Option<&str>, service: &str, op: &str, duration_us: i64) -> Span {
        let now = Utc::now();
        Span {
            trace_id: trace_id.to_string(),
            span_id: span_id.to_string(),
            parent_span_id: parent.map(|s| s.to_string()),
            operation_name: op.to_string(),
            service_name: service.to_string(),
            start_time: now,
            end_time: now,
            duration_us,
            status: SpanStatus::Unset,
            kind: SpanKind::Internal,
            tags: HashMap::new(),
            events: vec![],
            links: vec![],
            resource_attributes: HashMap::new(),
        }
    }

    #[test]
    fn span_is_root() {
        let span = make_span("t1", "s1", None, "svc", "op", 1000);
        assert!(span.is_root());
        let child = make_span("t1", "s2", Some("s1"), "svc", "op2", 500);
        assert!(!child.is_root());
    }

    #[test]
    fn trace_from_spans() {
        let root = make_span("t1", "s1", None, "frontend", "http.request", 5000);
        let child1 = make_span("t1", "s2", Some("s1"), "backend", "db.query", 2000);
        let child2 = make_span("t1", "s3", Some("s1"), "cache", "cache.get", 100);
        let trace = Trace::from_spans(vec![root, child1, child2]).unwrap();
        assert_eq!(trace.trace_id, "t1");
        assert_eq!(trace.service_name, "frontend");
        assert_eq!(trace.span_count, 3);
        assert_eq!(trace.duration_us, 5000);
    }
}
