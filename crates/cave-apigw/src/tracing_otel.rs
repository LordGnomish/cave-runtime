// SPDX-License-Identifier: AGPL-3.0-or-later
//! OpenTelemetry tracing spans + propagation.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SpanCtx {
    pub trace_id: String, pub span_id: String, pub parent_span_id: Option<String>,
    pub baggage: HashMap<String, String>,
}
impl SpanCtx {
    pub fn root() -> Self {
        Self { trace_id: random_hex(32), span_id: random_hex(16), parent_span_id: None, baggage: HashMap::new() }
    }
    pub fn child(&self) -> Self {
        Self { trace_id: self.trace_id.clone(), span_id: random_hex(16),
            parent_span_id: Some(self.span_id.clone()), baggage: self.baggage.clone() }
    }
    pub fn traceparent_header(&self) -> String {
        let parent = self.parent_span_id.clone().unwrap_or_else(|| "0000000000000000".into());
        format!("00-{}-{}-01", self.trace_id, parent)
    }
    pub fn from_traceparent(value: &str) -> Option<Self> {
        let parts: Vec<&str> = value.split('-').collect();
        if parts.len() != 4 || parts[0] != "00" { return None; }
        Some(Self { trace_id: parts[1].into(), span_id: random_hex(16),
            parent_span_id: Some(parts[2].into()), baggage: HashMap::new() })
    }
}

fn random_hex(n: usize) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos().to_le_bytes());
    h.update(format!("{:p}", &n).as_bytes());
    let d = h.finalize();
    hex::encode(&d[..n / 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn root_has_no_parent() {
        let s = SpanCtx::root();
        assert!(s.parent_span_id.is_none());
        assert_eq!(s.trace_id.len(), 32);
        assert_eq!(s.span_id.len(), 16);
    }
    #[test] fn child_inherits_trace() {
        let r = SpanCtx::root(); let c = r.child();
        assert_eq!(c.trace_id, r.trace_id);
        assert_eq!(c.parent_span_id.as_ref(), Some(&r.span_id));
        assert_ne!(c.span_id, r.span_id);
    }
    #[test] fn traceparent_format() {
        let r = SpanCtx::root();
        let tp = r.traceparent_header();
        assert!(tp.starts_with("00-"));
        assert!(tp.ends_with("-01"));
    }
    #[test] fn parse_traceparent() {
        let tp = "00-0af7651916cd43dd8448eb211c80319c-b9c7c989f97918e1-01";
        let c = SpanCtx::from_traceparent(tp).unwrap();
        assert_eq!(c.trace_id, "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(c.parent_span_id.as_deref(), Some("b9c7c989f97918e1"));
    }
    #[test] fn parse_traceparent_bad() {
        assert!(SpanCtx::from_traceparent("xx-1-2-3").is_none());
        assert!(SpanCtx::from_traceparent("00-a-b").is_none());
    }
}
