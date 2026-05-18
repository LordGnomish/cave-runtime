// SPDX-License-Identifier: AGPL-3.0-or-later
//! W3C Trace-Context propagation (`traceparent` + `tracestate`).
//!
//! Mirrors the strict spec semantics: rejects version `ff`, all-zero IDs,
//! malformed fields. `tracestate` enforces 32-entry HEAD-of-list cap and
//! validates keys/values against the W3C grammar.

use crate::types::{format_span_id, format_trace_id, parse_span_id, parse_trace_id, SpanContext};
use thiserror::Error;

/// The standard header name for the W3C traceparent.
pub const TRACEPARENT: &str = "traceparent";

/// The standard header name for the W3C tracestate.
pub const TRACESTATE: &str = "tracestate";

/// The current supported version of the traceparent header (0x00).
pub const VERSION: u8 = 0x00;

/// The sampled flag bit in the traceparent flags byte.
pub const FLAG_SAMPLED: u8 = SpanContext::FLAG_SAMPLED;

/// The maximum number of key-value pairs allowed in a tracestate header.
pub const MAX_TRACESTATE_ENTRIES: usize = 32;

/// Errors that can occur during trace context propagation.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PropagationError {
     /// The traceparent header did not contain exactly four fields.
     #[error("traceparent has wrong field count")]
    WrongFieldCount,
     /// The traceparent version field is invalid.
     #[error("traceparent version invalid: {0}")]
    InvalidVersion(String),
     /// The traceparent trace_id field is invalid.
     #[error("traceparent trace_id invalid: {0}")]
    InvalidTraceId(String),
     /// The traceparent span_id field is invalid.
     #[error("traceparent span_id invalid: {0}")]
    InvalidSpanId(String),
     /// The traceparent flags field is invalid.
     #[error("traceparent flags invalid: {0}")]
    InvalidFlags(String),
     /// The traceparent trace_id is all zeros.
     #[error("traceparent trace_id is all zeros")]
    ZeroTraceId,
     /// The traceparent span_id is all zeros.
     #[error("traceparent span_id is all zeros")]
    ZeroSpanId,
}

/// A result type for propagation operations.
pub type PropagationResult<T> = std::result::Result<T, PropagationError>;

/// Strict parser for the `traceparent` header. Returns `Err` for any
/// deviation from W3C spec.
pub fn parse_traceparent(header: &str) -> PropagationResult<SpanContext> {
    let header = header.trim();
    let parts: Vec<&str> = header.split('-').collect();
    if parts.len() != 4 {
        return Err(PropagationError::WrongFieldCount);
    }
    if parts[0].len() != 2 {
        return Err(PropagationError::InvalidVersion(parts[0].to_string()));
    }
    let version = u8::from_str_radix(parts[0], 16)
         .map_err(|_| PropagationError::InvalidVersion(parts[0].to_string()))?;
    if version == 0xff {
        return Err(PropagationError::InvalidVersion("ff".into()));
    }

    if parts[1].len() != 32 {
        return Err(PropagationError::InvalidTraceId(parts[1].to_string()));
    }
    let trace_id = parse_trace_id(parts[1])
         .ok_or_else(|| PropagationError::InvalidTraceId(parts[1].to_string()))?;
    if trace_id == 0 {
        return Err(PropagationError::ZeroTraceId);
    }

    if parts[2].len() != 16 {
        return Err(PropagationError::InvalidSpanId(parts[2].to_string()));
    }
    let span_id = parse_span_id(parts[2])
         .ok_or_else(|| PropagationError::InvalidSpanId(parts[2].to_string()))?;
    if span_id == 0 {
        return Err(PropagationError::ZeroSpanId);
    }

    if parts[3].len() != 2 {
        return Err(PropagationError::InvalidFlags(parts[3].to_string()));
    }
    let flags = u8::from_str_radix(parts[3], 16)
         .map_err(|_| PropagationError::InvalidFlags(parts[3].to_string()))?;

    Ok(SpanContext { trace_id, span_id, trace_flags: flags, is_remote: true })
}

/// Render a `traceparent` from a SpanContext.
pub fn format_traceparent(ctx: &SpanContext) -> String {
    format!(
         "{:02x}-{}-{}-{:02x}",
        VERSION,
        format_trace_id(ctx.trace_id),
        format_span_id(ctx.span_id),
        ctx.trace_flags
     )
}

/// Represents the W3C TraceState header content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceState {
     /// HEAD-of-list precedence: index 0 is the most-recent vendor.
    pub entries: Vec<(String, String)>,
}

impl TraceState {
    /// Creates a new, empty TraceState.
    pub fn new() -> Self { Default::default() }

    /// Inserts or updates a key-value pair, enforcing the 32-entry limit.
    pub fn upsert(&mut self, key: &str, value: &str) {
        self.entries.retain(|(k, _)| k != key);
        self.entries.insert(0, (key.to_string(), value.to_string()));
        self.entries.truncate(MAX_TRACESTATE_ENTRIES);
    }

    /// Retrieves the value for a given key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Serializes the TraceState into a header string.
    pub fn to_header(&self) -> String {
        self.entries.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join(",")
     }
}

/// Parses a `tracestate` header string into a TraceState struct.
pub fn parse_tracestate(header: &str) -> TraceState {
    let mut entries = Vec::new();
    for raw in header.split(',') {
        let trimmed = raw.trim();
        if trimmed.is_empty() { continue; }
        let mut parts = trimmed.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        let value = parts.next().unwrap_or("").trim();
        if !is_valid_key(key) || !is_valid_value(value) { continue; }
        entries.push((key.to_string(), value.to_string()));
        if entries.len() == MAX_TRACESTATE_ENTRIES { break; }
    }
    TraceState { entries }
}

/// Validates a tracestate key against W3C grammar rules.
fn is_valid_key(k: &str) -> bool {
    if k.is_empty() || k.len() > 256 { return false; }
    let bytes = k.as_bytes();
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() { return false; }
    bytes.iter().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit()
             || matches!(b, b'_' | b'-' | b'*' | b'/' | b'@')
     })
}

/// Validates a tracestate value against W3C grammar rules.
fn is_valid_value(v: &str) -> bool {
    if v.is_empty() || v.len() > 256 { return false; }
    v.bytes().all(|b| (0x20..=0x7e).contains(&b) && b != b',' && b != b'=')
}

/// Lossy extract — returns a fresh sampled context if the header is missing
/// or malformed. Used by SDK ingest paths that MUST always have a context.
pub fn extract_or_new(traceparent: Option<&str>, tracestate: Option<&str>) -> (SpanContext, TraceState) {
    let ctx = traceparent
         .and_then(|h| parse_traceparent(h).ok())
         .unwrap_or_else(|| SpanContext {
            trace_id: crate::id::new_trace_id(),
            span_id: crate::id::new_span_id(),
            trace_flags: FLAG_SAMPLED,
            is_remote: false,
         });
    let state = tracestate.map(parse_tracestate).unwrap_or_default();
    (ctx, state)
}

/// Injects trace context into header strings.
pub fn inject(ctx: &SpanContext, state: &TraceState) -> (String, String) {
    (format_traceparent(ctx), state.to_header())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "00-0af7651916cd43dd8448eb211c80319c-b9c7c989f97918e1-01";

    #[test]
    fn test_parse_canonical() {
        let c = parse_traceparent(SAMPLE).unwrap();
        assert_eq!(c.trace_id, 0x0af7651916cd43dd8448eb211c80319c);
        assert_eq!(c.span_id, 0xb9c7c989f97918e1);
        assert!(c.is_sampled());
        assert!(c.is_remote);
    }

    #[test]
    fn test_format_round_trip() {
        let c = parse_traceparent(SAMPLE).unwrap();
        // is_remote round-trip: format doesn't preserve it
        assert_eq!(format_traceparent(&c), SAMPLE);
    }

    #[test]
    fn test_unsampled_flag() {
        let c = parse_traceparent("00-0af7651916cd43dd8448eb211c80319c-b9c7c989f97918e1-00").unwrap();
        assert!(!c.is_sampled());
    }

    #[test]
    fn test_rejects_wrong_field_count() {
        assert_eq!(parse_traceparent("00-aa-bb").unwrap_err(), PropagationError::WrongFieldCount);
    }

    #[test]
    fn test_rejects_short_trace_id() {
        let err = parse_traceparent("00-aa-b9c7c989f97918e1-01").unwrap_err();
        assert!(matches!(err, PropagationError::InvalidTraceId(_)));
    }

    #[test]
    fn test_rejects_short_span_id() {
        let err = parse_traceparent("00-0af7651916cd43dd8448eb211c80319c-aa-01").unwrap_err();
        assert!(matches!(err, PropagationError::InvalidSpanId(_)));
    }

    #[test]
    fn test_rejects_zero_trace_id() {
        let h = "00-00000000000000000000000000000000-b9c7c989f97918e1-01";
        assert_eq!(parse_traceparent(h).unwrap_err(), PropagationError::ZeroTraceId);
    }

    #[test]
    fn test_rejects_zero_span_id() {
        let h = "00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01";
        assert_eq!(parse_traceparent(h).unwrap_err(), PropagationError::ZeroSpanId);
    }

    #[test]
    fn test_rejects_version_ff() {
        let h = "ff-0af7651916cd43dd8448eb211c80319c-b9c7c989f97918e1-01";
        assert!(matches!(parse_traceparent(h).unwrap_err(), PropagationError::InvalidVersion(_)));
    }

    #[test]
    fn test_tracestate_basic() {
        let s = parse_tracestate("congo=t61rcWkgMzE,rojo=00f067aa0ba902b7");
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.get("congo"), Some("t61rcWkgMzE"));
    }

    #[test]
    fn test_tracestate_drops_malformed() {
        let s = parse_tracestate("=value,vendor=ok,bad=");
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.get("vendor"), Some("ok"));
    }

    #[test]
    fn test_tracestate_caps_at_32() {
        let huge: String = (0..40).map(|i| format!("k{}=v{}", i, i)).collect::<Vec<_>>().join(",");
        let s = parse_tracestate(&huge);
        assert_eq!(s.entries.len(), MAX_TRACESTATE_ENTRIES);
    }

    #[test]
    fn test_tracestate_upsert_head() {
        let mut s = parse_tracestate("a=1,b=2,c=3");
        s.upsert("b", "20");
        assert_eq!(s.entries[0], ("b".into(), "20".into()));
    }

    #[test]
    fn test_tracestate_upsert_new_at_head() {
        let mut s = parse_tracestate("a=1");
        s.upsert("z", "9");
        assert_eq!(s.entries[0].0, "z");
    }

    #[test]
    fn test_extract_or_new_falls_back_when_missing() {
        let (c, _s) = extract_or_new(None, None);
        assert!(c.is_valid());
        assert!(c.is_sampled());
        assert!(!c.is_remote);
    }

    #[test]
    fn test_extract_or_new_falls_back_on_malformed() {
        let (c, _s) = extract_or_new(Some("garbage"), None);
        assert!(c.is_valid());
    }

    #[test]
    fn test_inject_round_trip() {
        let mut s = TraceState::new();
        s.upsert("vendor", "value");
        let ctx = SpanContext::new(0xdeadbeef, 0xcafe, true);
        let (tp, ts) = inject(&ctx, &s);
        let parsed_ctx = parse_traceparent(&tp).unwrap();
        assert_eq!(parsed_ctx.trace_id, ctx.trace_id);
        assert_eq!(parsed_ctx.span_id, ctx.span_id);
        assert_eq!(parse_tracestate(&ts), s);
    }

    #[test]
    fn test_validators() {
        assert!(is_valid_key("vendor"));
        assert!(is_valid_key("rojo@vendor"));
        assert!(!is_valid_key("UPPER"));
        assert!(!is_valid_key(""));
        assert!(is_valid_value("00f067aa"));
        assert!(!is_valid_value(""));
        assert!(!is_valid_value("with=eq"));
    }
}
