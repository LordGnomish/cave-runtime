// SPDX-License-Identifier: AGPL-3.0-or-later
//! W3C Trace Context propagation.
//!
//! Implements the [W3C trace-context recommendation](https://www.w3.org/TR/trace-context/)
//! for both `traceparent` and `tracestate` headers.
//!
//! `traceparent` format:
//!   `version-trace_id-parent_id-trace_flags`
//!   `00-0af7651916cd43dd8448eb211c80319c-b9c7c989f97918e1-01`
//!
//! `tracestate` format:
//!   `key1=value1,key2=value2[,...]`
//!   - max 32 entries (HEAD-of-list precedence)
//!   - max 512 chars total when formatted
//!   - keys: lowercase alpha + digit + `_-*/` + optional `@vendor`
//!   - values: 7-bit ASCII printable, no `,` or `=`
//!
//! Per spec we MUST drop malformed headers rather than refuse the trace; the
//! API surfaces both `try_parse_*` (returns Result) and `extract_or_new` (lossy)
//! variants so callers can choose between strict ingest and best-effort propagation.

use crate::types::{format_span_id, format_trace_id, parse_span_id, parse_trace_id, SpanId, TraceId};
use thiserror::Error;

pub const VERSION: u8 = 0x00;
pub const FLAG_SAMPLED: u8 = 0x01;

pub const TRACEPARENT: &str = "traceparent";
pub const TRACESTATE: &str = "tracestate";

pub const MAX_TRACESTATE_ENTRIES: usize = 32;
pub const MAX_TRACESTATE_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceParent {
    pub version: u8,
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub flags: u8,
}

impl TraceParent {
    pub fn new(trace_id: TraceId, span_id: SpanId, sampled: bool) -> Self {
        TraceParent {
            version: VERSION,
            trace_id,
            span_id,
            flags: if sampled { FLAG_SAMPLED } else { 0 },
        }
    }

    pub fn is_sampled(&self) -> bool {
        self.flags & FLAG_SAMPLED == FLAG_SAMPLED
    }

    /// Serialize to the wire format.
    pub fn to_header(&self) -> String {
        format!(
            "{:02x}-{}-{}-{:02x}",
            self.version,
            format_trace_id(self.trace_id),
            format_span_id(self.span_id),
            self.flags
        )
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PropagationError {
    #[error("traceparent has wrong field count")]
    WrongFieldCount,
    #[error("traceparent version invalid: {0}")]
    InvalidVersion(String),
    #[error("traceparent trace_id invalid: {0}")]
    InvalidTraceId(String),
    #[error("traceparent span_id invalid: {0}")]
    InvalidSpanId(String),
    #[error("traceparent flags invalid: {0}")]
    InvalidFlags(String),
    #[error("traceparent trace_id is all zeros")]
    ZeroTraceId,
    #[error("traceparent span_id is all zeros")]
    ZeroSpanId,
    #[error("tracestate entry malformed: {0}")]
    InvalidStateEntry(String),
}

pub type PropagationResult<T> = std::result::Result<T, PropagationError>;

/// Strict parser. Returns `Err` for any deviation from the spec.
pub fn parse_traceparent(header: &str) -> PropagationResult<TraceParent> {
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
    // Spec: version FF is forbidden
    if version == 0xff {
        return Err(PropagationError::InvalidVersion("ff".into()));
    }

    if parts[1].len() != 32 {
        return Err(PropagationError::InvalidTraceId(parts[1].to_string()));
    }
    let trace_id = parse_trace_id(parts[1])
        .map_err(|_| PropagationError::InvalidTraceId(parts[1].to_string()))?;
    if trace_id == 0 {
        return Err(PropagationError::ZeroTraceId);
    }

    if parts[2].len() != 16 {
        return Err(PropagationError::InvalidSpanId(parts[2].to_string()));
    }
    let span_id = parse_span_id(parts[2])
        .map_err(|_| PropagationError::InvalidSpanId(parts[2].to_string()))?;
    if span_id == 0 {
        return Err(PropagationError::ZeroSpanId);
    }

    if parts[3].len() != 2 {
        return Err(PropagationError::InvalidFlags(parts[3].to_string()));
    }
    let flags = u8::from_str_radix(parts[3], 16)
        .map_err(|_| PropagationError::InvalidFlags(parts[3].to_string()))?;

    Ok(TraceParent { version, trace_id, span_id, flags })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceState {
    /// HEAD-of-list precedence: index 0 is the most-recent vendor.
    pub entries: Vec<(String, String)>,
}

impl TraceState {
    pub fn new() -> Self {
        Default::default()
    }

    /// Insert a vendor entry, moving it to the head if it already existed.
    pub fn upsert(&mut self, key: &str, value: &str) {
        self.entries.retain(|(k, _)| k != key);
        self.entries.insert(0, (key.to_string(), value.to_string()));
        // Cap at 32 entries (drop tail per spec)
        self.entries.truncate(MAX_TRACESTATE_ENTRIES);
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn to_header(&self) -> String {
        self.entries
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Strict parser for `tracestate`. Drops entries that are malformed
/// individually rather than failing the whole header (per spec).
pub fn parse_tracestate(header: &str) -> TraceState {
    let mut entries = Vec::new();
    for raw in header.split(',') {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        let value = parts.next().unwrap_or("").trim();
        if !is_valid_key(key) || !is_valid_value(value) {
            continue;
        }
        entries.push((key.to_string(), value.to_string()));
        if entries.len() == MAX_TRACESTATE_ENTRIES {
            break;
        }
    }
    TraceState { entries }
}

fn is_valid_key(k: &str) -> bool {
    if k.is_empty() || k.len() > 256 {
        return false;
    }
    let bytes = k.as_bytes();
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return false;
    }
    bytes.iter().all(|b| {
        b.is_ascii_lowercase()
            || b.is_ascii_digit()
            || matches!(b, b'_' | b'-' | b'*' | b'/' | b'@')
    })
}

fn is_valid_value(v: &str) -> bool {
    if v.is_empty() || v.len() > 256 {
        return false;
    }
    v.bytes().all(|b| (0x20..=0x7e).contains(&b) && b != b',' && b != b'=')
}

/// Convenience: extract from a header map, returning a fresh trace if missing
/// or malformed. Useful for the OTLP/HTTP entry-points where we MUST always
/// produce a span context.
pub fn extract_or_new(traceparent: Option<&str>, tracestate: Option<&str>) -> (TraceParent, TraceState) {
    let parent = traceparent
        .and_then(|h| parse_traceparent(h).ok())
        .unwrap_or_else(|| {
            // Fall back to a fresh sampled trace
            TraceParent::new(rand_u128(), rand_u64(), true)
        });
    let state = tracestate.map(parse_tracestate).unwrap_or_default();
    (parent, state)
}

/// Inject the current parent + state into outbound headers, returning
/// `(traceparent, tracestate)`.
pub fn inject(parent: &TraceParent, state: &TraceState) -> (String, String) {
    (parent.to_header(), state.to_header())
}

fn rand_u128() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    // tiny non-crypto RNG: time + thread id mixed via FNV. The runtime
    // uses a real RNG when available; this fallback keeps the propagation
    // module dependency-free.
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let id = std::thread::current().id();
    let mix = format!("{:?}-{}", id, t);
    let mut h: u128 = 0xcbf29ce4_84222325_cbf29ce4_84222325;
    for b in mix.as_bytes() {
        h ^= *b as u128;
        h = h.wrapping_mul(0x00000100_000001b3);
    }
    if h == 0 { 1 } else { h }
}

fn rand_u64() -> u64 {
    let mix = rand_u128();
    let lo = mix as u64;
    let hi = (mix >> 64) as u64;
    let v = lo ^ hi;
    if v == 0 { 1 } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "00-0af7651916cd43dd8448eb211c80319c-b9c7c989f97918e1-01";

    #[test]
    fn test_parse_canonical_traceparent() {
        let p = parse_traceparent(SAMPLE).unwrap();
        assert_eq!(p.version, 0);
        assert_eq!(p.trace_id, 0x0af7651916cd43dd8448eb211c80319c);
        assert_eq!(p.span_id, 0xb9c7c989f97918e1);
        assert!(p.is_sampled());
    }

    #[test]
    fn test_traceparent_to_header_round_trip() {
        let p = parse_traceparent(SAMPLE).unwrap();
        assert_eq!(p.to_header(), SAMPLE);
    }

    #[test]
    fn test_traceparent_unsampled_flag() {
        let p = parse_traceparent("00-0af7651916cd43dd8448eb211c80319c-b9c7c989f97918e1-00").unwrap();
        assert!(!p.is_sampled());
    }

    #[test]
    fn test_traceparent_rejects_wrong_field_count() {
        assert_eq!(
            parse_traceparent("00-aaaa-bbbb").unwrap_err(),
            PropagationError::WrongFieldCount
        );
    }

    #[test]
    fn test_traceparent_rejects_short_trace_id() {
        let err = parse_traceparent("00-0af-b9c7c989f97918e1-01").unwrap_err();
        assert!(matches!(err, PropagationError::InvalidTraceId(_)));
    }

    #[test]
    fn test_traceparent_rejects_short_span_id() {
        let err = parse_traceparent("00-0af7651916cd43dd8448eb211c80319c-b9c-01").unwrap_err();
        assert!(matches!(err, PropagationError::InvalidSpanId(_)));
    }

    #[test]
    fn test_traceparent_rejects_zero_trace_id() {
        let zero_trace = "00-00000000000000000000000000000000-b9c7c989f97918e1-01";
        assert_eq!(parse_traceparent(zero_trace).unwrap_err(), PropagationError::ZeroTraceId);
    }

    #[test]
    fn test_traceparent_rejects_zero_span_id() {
        let zero_span = "00-0af7651916cd43dd8448eb211c80319c-0000000000000000-01";
        assert_eq!(parse_traceparent(zero_span).unwrap_err(), PropagationError::ZeroSpanId);
    }

    #[test]
    fn test_traceparent_rejects_version_ff() {
        let h = "ff-0af7651916cd43dd8448eb211c80319c-b9c7c989f97918e1-01";
        assert!(matches!(parse_traceparent(h).unwrap_err(), PropagationError::InvalidVersion(_)));
    }

    #[test]
    fn test_traceparent_new_with_sampled_flag() {
        let p = TraceParent::new(0xdeadbeef, 0xcafe, true);
        assert!(p.is_sampled());
        let p2 = TraceParent::new(0xdeadbeef, 0xcafe, false);
        assert!(!p2.is_sampled());
    }

    #[test]
    fn test_tracestate_parses_two_entries() {
        let s = parse_tracestate("congo=t61rcWkgMzE,rojo=00f067aa0ba902b7");
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.entries[0].0, "congo");
        assert_eq!(s.entries[1].0, "rojo");
    }

    #[test]
    fn test_tracestate_drops_malformed_entries() {
        // First malformed (empty key), second OK, third malformed (no value)
        let s = parse_tracestate("=value,vendor=ok,bad=");
        // empty value not allowed → bad= dropped, =value dropped, only vendor=ok kept
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.get("vendor"), Some("ok"));
    }

    #[test]
    fn test_tracestate_caps_at_32_entries() {
        let huge: String = (0..40).map(|i| format!("k{}=v{}", i, i)).collect::<Vec<_>>().join(",");
        let s = parse_tracestate(&huge);
        assert_eq!(s.entries.len(), MAX_TRACESTATE_ENTRIES);
    }

    #[test]
    fn test_tracestate_upsert_moves_to_head() {
        let mut s = parse_tracestate("a=1,b=2,c=3");
        s.upsert("b", "20");
        assert_eq!(s.entries[0], ("b".to_string(), "20".to_string()));
        assert_eq!(s.entries.len(), 3);
    }

    #[test]
    fn test_tracestate_upsert_inserts_new_at_head() {
        let mut s = parse_tracestate("a=1");
        s.upsert("z", "9");
        assert_eq!(s.entries[0].0, "z");
        assert_eq!(s.entries[1].0, "a");
    }

    #[test]
    fn test_tracestate_to_header_round_trip() {
        let s = parse_tracestate("congo=t61rcWkgMzE,rojo=00f067aa0ba902b7");
        assert_eq!(s.to_header(), "congo=t61rcWkgMzE,rojo=00f067aa0ba902b7");
    }

    #[test]
    fn test_tracestate_empty_header_yields_empty_state() {
        let s = parse_tracestate("");
        assert!(s.entries.is_empty());
    }

    #[test]
    fn test_tracestate_skips_whitespace_only_segments() {
        let s = parse_tracestate("  ,a=1, ,b=2,  ");
        assert_eq!(s.entries.len(), 2);
    }

    #[test]
    fn test_tracestate_rejects_uppercase_key() {
        let s = parse_tracestate("ABC=1,ok=2");
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.get("ok"), Some("2"));
    }

    #[test]
    fn test_tracestate_rejects_value_with_comma() {
        // Value with comma cannot reach the entry parser intact (pre-split),
        // but a value containing `=` would. Check that.
        let s = parse_tracestate("k=a=b");
        // value "a=b" has '=' which is forbidden in spec
        assert!(s.entries.is_empty());
    }

    #[test]
    fn test_tracestate_accepts_vendor_at_key() {
        let s = parse_tracestate("rojo@vendor=00f067aa");
        assert_eq!(s.entries.len(), 1);
    }

    #[test]
    fn test_extract_or_new_falls_back_when_missing() {
        let (p, s) = extract_or_new(None, None);
        assert_ne!(p.trace_id, 0);
        assert_ne!(p.span_id, 0);
        assert!(p.is_sampled());
        assert!(s.entries.is_empty());
    }

    #[test]
    fn test_extract_or_new_falls_back_on_malformed() {
        let (p, _s) = extract_or_new(Some("garbage"), None);
        assert_ne!(p.trace_id, 0);
    }

    #[test]
    fn test_extract_or_new_uses_provided() {
        let (p, _s) = extract_or_new(Some(SAMPLE), None);
        assert_eq!(p.trace_id, 0x0af7651916cd43dd8448eb211c80319c);
    }

    #[test]
    fn test_inject_round_trips_traceparent_and_tracestate() {
        let p = TraceParent::new(0x0af7651916cd43dd8448eb211c80319c, 0xb9c7c989f97918e1, true);
        let mut s = TraceState::new();
        s.upsert("congo", "t61rcWkgMzE");
        let (tp, ts) = inject(&p, &s);
        assert_eq!(parse_traceparent(&tp).unwrap(), p);
        assert_eq!(parse_tracestate(&ts), s);
    }

    #[test]
    fn test_is_valid_key_examples() {
        assert!(is_valid_key("vendor"));
        assert!(is_valid_key("rojo@vendor"));
        assert!(is_valid_key("a-b_c/d*e"));
        assert!(!is_valid_key(""));
        assert!(!is_valid_key("UPPER"));
        assert!(!is_valid_key("@vendor"));
    }

    #[test]
    fn test_is_valid_value_examples() {
        assert!(is_valid_value("00f067aa0ba902b7"));
        assert!(is_valid_value("a:b:c"));
        assert!(!is_valid_value("with,comma"));
        assert!(!is_valid_value("with=equals"));
        assert!(!is_valid_value(""));
    }

    #[test]
    fn test_traceparent_round_trip_random_ids() {
        for (tid, sid) in [(1u128, 1u64), (u128::MAX, u64::MAX), (0xdeadbeefcafebabe, 0x1234)] {
            let p = TraceParent::new(tid, sid, true);
            assert_eq!(parse_traceparent(&p.to_header()).unwrap(), p);
        }
    }

    #[test]
    fn test_random_ids_are_nonzero() {
        for _ in 0..50 {
            assert_ne!(rand_u128(), 0);
            assert_ne!(rand_u64(), 0);
        }
    }
}
