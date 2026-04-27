//! Trace propagation — W3C TraceContext + Zipkin B3 + B3-single-header.
//!
//! The gateway sits at the edge: every request gets a trace context, either
//! by parsing inbound headers or by minting a fresh one. Outbound headers
//! are then re-serialized for whichever propagator the upstream expects.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── TraceContext — the canonical in-process representation ─────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceContext {
    /// 16-byte trace ID (32 lowercase hex chars).
    pub trace_id: [u8; 16],
    /// 8-byte span ID (16 lowercase hex chars).
    pub span_id: [u8; 8],
    /// Optional parent span (used as the new span's parent).
    pub parent_span_id: Option<[u8; 8]>,
    /// Sampled flag (`flags & 0x01`).
    pub sampled: bool,
    /// W3C `tracestate` raw value, opaque to us; we propagate it.
    pub trace_state: String,
}

impl TraceContext {
    pub fn root(trace_id: [u8; 16], span_id: [u8; 8]) -> Self {
        Self {
            trace_id,
            span_id,
            parent_span_id: None,
            sampled: true,
            trace_state: String::new(),
        }
    }

    pub fn trace_id_hex(&self) -> String {
        hex_encode(&self.trace_id)
    }

    pub fn span_id_hex(&self) -> String {
        hex_encode(&self.span_id)
    }

    pub fn parent_span_id_hex(&self) -> Option<String> {
        self.parent_span_id.map(|s| hex_encode(&s))
    }

    pub fn flags_byte(&self) -> u8 {
        if self.sampled { 0x01 } else { 0x00 }
    }
}

fn hex_encode(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() * 2);
    for byte in b {
        out.push(hex_digit((byte >> 4) & 0xf));
        out.push(hex_digit(byte & 0xf));
    }
    out
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => '?',
    }
}

fn hex_decode<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for i in 0..N {
        let hi = hex_value(s.as_bytes()[i * 2])?;
        let lo = hex_value(s.as_bytes()[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ── W3C TraceContext (RFC 9001-flavoured `traceparent` / `tracestate`) ─────

/// Parse a W3C `traceparent` header value. Format:
/// `<version>-<trace-id>-<parent-id>-<flags>` all lowercase hex.
pub fn parse_traceparent(value: &str, tracestate: Option<&str>) -> Option<TraceContext> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 4 {
        return None;
    }
    if parts[0] != "00" {
        // Future versions: be liberal (Envoy does too) and try to interpret;
        // here we accept only "00".
        return None;
    }
    if parts[1].len() != 32 || parts[2].len() != 16 || parts[3].len() != 2 {
        return None;
    }
    let trace_id = hex_decode::<16>(parts[1])?;
    if trace_id == [0u8; 16] {
        return None;
    }
    let parent = hex_decode::<8>(parts[2])?;
    if parent == [0u8; 8] {
        return None;
    }
    let flags = u8::from_str_radix(parts[3], 16).ok()?;
    Some(TraceContext {
        trace_id,
        span_id: parent,
        parent_span_id: None,
        sampled: flags & 0x01 != 0,
        trace_state: tracestate.unwrap_or("").to_string(),
    })
}

pub fn render_traceparent(ctx: &TraceContext) -> String {
    format!(
        "00-{}-{}-{:02x}",
        ctx.trace_id_hex(),
        ctx.span_id_hex(),
        ctx.flags_byte()
    )
}

// ── Zipkin B3 (multi-header) ─────────────────────────────────────────────────

/// Parse the multi-header Zipkin B3 propagator: `X-B3-TraceId`,
/// `X-B3-SpanId`, optional `X-B3-ParentSpanId`, `X-B3-Sampled`.
pub fn parse_b3_multi(headers: &HashMap<String, String>) -> Option<TraceContext> {
    let raw_trace = headers.get("x-b3-traceid")?.as_str();
    let trace_id: [u8; 16] = match raw_trace.len() {
        32 => hex_decode::<16>(raw_trace)?,
        16 => {
            // 64-bit trace IDs → left-pad with zeros.
            let mut bytes = [0u8; 16];
            let bottom = hex_decode::<8>(raw_trace)?;
            bytes[8..].copy_from_slice(&bottom);
            bytes
        }
        _ => return None,
    };
    if trace_id == [0u8; 16] {
        return None;
    }
    let span_id = hex_decode::<8>(headers.get("x-b3-spanid")?.as_str())?;
    if span_id == [0u8; 8] {
        return None;
    }
    let parent = headers
        .get("x-b3-parentspanid")
        .and_then(|s| hex_decode::<8>(s.as_str()));
    let sampled = match headers.get("x-b3-sampled").map(|s| s.as_str()) {
        Some("1") | Some("true") | Some("d") => true,
        Some("0") | Some("false") => false,
        _ => true,
    };
    Some(TraceContext {
        trace_id,
        span_id,
        parent_span_id: parent,
        sampled,
        trace_state: String::new(),
    })
}

pub fn render_b3_multi(ctx: &TraceContext) -> Vec<(&'static str, String)> {
    let mut out = vec![
        ("x-b3-traceid", ctx.trace_id_hex()),
        ("x-b3-spanid", ctx.span_id_hex()),
        ("x-b3-sampled", if ctx.sampled { "1" } else { "0" }.to_string()),
    ];
    if let Some(p) = ctx.parent_span_id_hex() {
        out.push(("x-b3-parentspanid", p));
    }
    out
}

// ── B3 single-header (`b3`) ─────────────────────────────────────────────────

/// `b3` single-header format: `<traceid>-<spanid>[-<sampled>][-<parentspanid>]`.
pub fn parse_b3_single(value: &str) -> Option<TraceContext> {
    if value == "0" {
        return Some(TraceContext {
            trace_id: [0u8; 16],
            span_id: [0u8; 8],
            parent_span_id: None,
            sampled: false,
            trace_state: String::new(),
        });
    }
    let parts: Vec<&str> = value.split('-').collect();
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    let trace_id: [u8; 16] = match parts[0].len() {
        32 => hex_decode::<16>(parts[0])?,
        16 => {
            let mut bytes = [0u8; 16];
            let b = hex_decode::<8>(parts[0])?;
            bytes[8..].copy_from_slice(&b);
            bytes
        }
        _ => return None,
    };
    let span_id = hex_decode::<8>(parts[1])?;
    let mut sampled = true;
    let mut parent: Option<[u8; 8]> = None;
    if parts.len() >= 3 {
        sampled = matches!(parts[2], "1" | "d");
    }
    if parts.len() == 4 {
        parent = hex_decode::<8>(parts[3]);
    }
    Some(TraceContext {
        trace_id,
        span_id,
        parent_span_id: parent,
        sampled,
        trace_state: String::new(),
    })
}

pub fn render_b3_single(ctx: &TraceContext) -> String {
    let mut out = format!("{}-{}", ctx.trace_id_hex(), ctx.span_id_hex());
    out.push_str(if ctx.sampled { "-1" } else { "-0" });
    if let Some(p) = ctx.parent_span_id_hex() {
        out.push('-');
        out.push_str(&p);
    }
    out
}

// ── Top-level extraction / injection ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Propagator {
    W3c,
    B3Multi,
    B3Single,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceConfig {
    /// Ordered propagators tried for extraction (first hit wins).
    pub extractors: Vec<Propagator>,
    /// Propagators that get written into the upstream request.
    pub injectors: Vec<Propagator>,
    /// Sampling rate 0.0–1.0; only matters when minting a fresh trace.
    pub sample_rate: f64,
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            extractors: vec![Propagator::W3c, Propagator::B3Multi, Propagator::B3Single],
            injectors: vec![Propagator::W3c, Propagator::B3Multi],
            sample_rate: 1.0,
        }
    }
}

impl TraceConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.extractors.is_empty() {
            return Err("at least one extractor required".into());
        }
        if !(0.0..=1.0).contains(&self.sample_rate) {
            return Err("sample_rate must be in [0,1]".into());
        }
        Ok(())
    }
}

/// Extract a trace context from inbound headers using the configured order.
pub fn extract(cfg: &TraceConfig, headers: &HashMap<String, String>) -> Option<TraceContext> {
    for p in &cfg.extractors {
        match p {
            Propagator::W3c => {
                if let Some(tp) = headers.get("traceparent") {
                    let ts = headers.get("tracestate").map(|s| s.as_str());
                    if let Some(ctx) = parse_traceparent(tp, ts) {
                        return Some(ctx);
                    }
                }
            }
            Propagator::B3Multi => {
                if let Some(ctx) = parse_b3_multi(headers) {
                    return Some(ctx);
                }
            }
            Propagator::B3Single => {
                if let Some(b3) = headers.get("b3") {
                    if let Some(ctx) = parse_b3_single(b3) {
                        return Some(ctx);
                    }
                }
            }
        }
    }
    None
}

/// Generate the headers to inject upstream, given a context. Returns a list
/// of (name, value) pairs; the caller merges into the outbound request.
pub fn inject(cfg: &TraceConfig, ctx: &TraceContext) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in &cfg.injectors {
        match p {
            Propagator::W3c => {
                out.push(("traceparent".into(), render_traceparent(ctx)));
                if !ctx.trace_state.is_empty() {
                    out.push(("tracestate".into(), ctx.trace_state.clone()));
                }
            }
            Propagator::B3Multi => {
                for (k, v) in render_b3_multi(ctx) {
                    out.push((k.into(), v));
                }
            }
            Propagator::B3Single => {
                out.push(("b3".into(), render_b3_single(ctx)));
            }
        }
    }
    out
}

/// Sampling decision based on sample_rate (deterministic — uses trace_id).
pub fn sample_decision(cfg: &TraceConfig, trace_id: &[u8; 16]) -> bool {
    if cfg.sample_rate <= 0.0 {
        return false;
    }
    if cfg.sample_rate >= 1.0 {
        return true;
    }
    // Take last 8 bytes as u64 and threshold against rate.
    let mut tail = [0u8; 8];
    tail.copy_from_slice(&trace_id[8..]);
    let v = u64::from_be_bytes(tail);
    let threshold = (cfg.sample_rate * (u64::MAX as f64)) as u64;
    v <= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), (*v).to_string()))
            .collect()
    }

    fn ctx(trace_hex: &str, span_hex: &str, sampled: bool) -> TraceContext {
        TraceContext {
            trace_id: hex_decode::<16>(trace_hex).unwrap(),
            span_id: hex_decode::<8>(span_hex).unwrap(),
            parent_span_id: None,
            sampled,
            trace_state: String::new(),
        }
    }

    // ── Hex round-trip ──

    #[test]
    fn hex_encode_decode_roundtrip_16() {
        let bytes: [u8; 16] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
        ];
        let hex = hex_encode(&bytes);
        assert_eq!(hex, "0102030405060708090a0b0c0d0e0f10");
        let back = hex_decode::<16>(&hex).unwrap();
        assert_eq!(back, bytes);
    }

    #[test]
    fn hex_decode_uppercase() {
        let bytes = hex_decode::<8>("DEADBEEFCAFEBABE").unwrap();
        assert_eq!(bytes, [0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe]);
    }

    #[test]
    fn hex_decode_wrong_length_fails() {
        assert!(hex_decode::<8>("abcd").is_none());
        assert!(hex_decode::<8>("abcdef0123456789a").is_none());
    }

    // ── W3C parse ──

    #[test]
    fn w3c_parse_well_formed() {
        let ctx = parse_traceparent(
            "00-0102030405060708090a0b0c0d0e0f10-0102030405060708-01",
            None,
        )
        .unwrap();
        assert_eq!(ctx.trace_id_hex(), "0102030405060708090a0b0c0d0e0f10");
        assert_eq!(ctx.span_id_hex(), "0102030405060708");
        assert!(ctx.sampled);
    }

    #[test]
    fn w3c_parse_unsampled() {
        let ctx = parse_traceparent(
            "00-0102030405060708090a0b0c0d0e0f10-0102030405060708-00",
            None,
        )
        .unwrap();
        assert!(!ctx.sampled);
    }

    #[test]
    fn w3c_parse_with_tracestate() {
        let ctx = parse_traceparent(
            "00-0102030405060708090a0b0c0d0e0f10-0102030405060708-01",
            Some("vendor=foo"),
        )
        .unwrap();
        assert_eq!(ctx.trace_state, "vendor=foo");
    }

    #[test]
    fn w3c_parse_wrong_version_rejects() {
        assert!(parse_traceparent(
            "01-0102030405060708090a0b0c0d0e0f10-0102030405060708-01",
            None
        )
        .is_none());
    }

    #[test]
    fn w3c_parse_zero_trace_rejects() {
        assert!(parse_traceparent(
            "00-00000000000000000000000000000000-0102030405060708-01",
            None
        )
        .is_none());
    }

    #[test]
    fn w3c_parse_zero_parent_rejects() {
        assert!(parse_traceparent(
            "00-0102030405060708090a0b0c0d0e0f10-0000000000000000-01",
            None
        )
        .is_none());
    }

    #[test]
    fn w3c_render_round_trip() {
        let c = ctx("0102030405060708090a0b0c0d0e0f10", "0102030405060708", true);
        let rendered = render_traceparent(&c);
        assert_eq!(
            rendered,
            "00-0102030405060708090a0b0c0d0e0f10-0102030405060708-01"
        );
        let back = parse_traceparent(&rendered, None).unwrap();
        assert_eq!(back.trace_id, c.trace_id);
        assert_eq!(back.span_id, c.span_id);
    }

    // ── B3 multi-header ──

    #[test]
    fn b3_multi_full_set_parses() {
        let h = hdr(&[
            ("X-B3-TraceId", "0102030405060708090a0b0c0d0e0f10"),
            ("X-B3-SpanId", "0102030405060708"),
            ("X-B3-ParentSpanId", "1112131415161718"),
            ("X-B3-Sampled", "1"),
        ]);
        let c = parse_b3_multi(&h).unwrap();
        assert_eq!(c.trace_id_hex(), "0102030405060708090a0b0c0d0e0f10");
        assert!(c.sampled);
        assert_eq!(c.parent_span_id_hex().unwrap(), "1112131415161718");
    }

    #[test]
    fn b3_multi_64bit_trace_pads_to_128() {
        let h = hdr(&[
            ("X-B3-TraceId", "0102030405060708"),
            ("X-B3-SpanId", "0102030405060708"),
        ]);
        let c = parse_b3_multi(&h).unwrap();
        assert_eq!(c.trace_id[..8], [0u8; 8]);
        assert_eq!(c.trace_id[8..], [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn b3_multi_sampled_default_when_missing() {
        let h = hdr(&[
            ("X-B3-TraceId", "0102030405060708090a0b0c0d0e0f10"),
            ("X-B3-SpanId", "0102030405060708"),
        ]);
        let c = parse_b3_multi(&h).unwrap();
        assert!(c.sampled);
    }

    #[test]
    fn b3_multi_zero_trace_rejects() {
        let h = hdr(&[
            ("X-B3-TraceId", "00000000000000000000000000000000"),
            ("X-B3-SpanId", "0102030405060708"),
        ]);
        assert!(parse_b3_multi(&h).is_none());
    }

    #[test]
    fn b3_multi_render_round_trip() {
        let mut c = ctx("0102030405060708090a0b0c0d0e0f10", "0102030405060708", false);
        c.parent_span_id = Some(hex_decode::<8>("1112131415161718").unwrap());
        let pairs = render_b3_multi(&c);
        let h: HashMap<String, String> = pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        assert_eq!(h["x-b3-sampled"], "0");
        assert_eq!(h["x-b3-parentspanid"], "1112131415161718");
    }

    // ── B3 single ──

    #[test]
    fn b3_single_short_form_rejected() {
        assert!(parse_b3_single("nope").is_none());
    }

    #[test]
    fn b3_single_two_field() {
        let c = parse_b3_single("0102030405060708090a0b0c0d0e0f10-0102030405060708").unwrap();
        assert_eq!(c.trace_id_hex(), "0102030405060708090a0b0c0d0e0f10");
        assert!(c.sampled);
    }

    #[test]
    fn b3_single_three_field_unsampled() {
        let c = parse_b3_single("0102030405060708090a0b0c0d0e0f10-0102030405060708-0").unwrap();
        assert!(!c.sampled);
    }

    #[test]
    fn b3_single_four_field_with_parent() {
        let c = parse_b3_single("0102030405060708090a0b0c0d0e0f10-0102030405060708-1-1112131415161718").unwrap();
        assert!(c.sampled);
        assert!(c.parent_span_id.is_some());
    }

    #[test]
    fn b3_single_disabled_zero() {
        let c = parse_b3_single("0").unwrap();
        assert!(!c.sampled);
        assert_eq!(c.trace_id, [0u8; 16]);
    }

    #[test]
    fn b3_single_render_round_trip() {
        let c = ctx("0102030405060708090a0b0c0d0e0f10", "0102030405060708", true);
        let rendered = render_b3_single(&c);
        assert_eq!(rendered, "0102030405060708090a0b0c0d0e0f10-0102030405060708-1");
    }

    // ── extract / inject ──

    #[test]
    fn extract_w3c_first() {
        let h = hdr(&[(
            "traceparent",
            "00-0102030405060708090a0b0c0d0e0f10-0102030405060708-01",
        )]);
        let cfg = TraceConfig::default();
        assert!(extract(&cfg, &h).is_some());
    }

    #[test]
    fn extract_falls_through_to_b3_multi() {
        let mut cfg = TraceConfig::default();
        cfg.extractors = vec![Propagator::W3c, Propagator::B3Multi];
        let h = hdr(&[
            ("X-B3-TraceId", "0102030405060708090a0b0c0d0e0f10"),
            ("X-B3-SpanId", "0102030405060708"),
        ]);
        assert!(extract(&cfg, &h).is_some());
    }

    #[test]
    fn extract_no_match_returns_none() {
        let cfg = TraceConfig::default();
        assert!(extract(&cfg, &HashMap::new()).is_none());
    }

    #[test]
    fn inject_emits_w3c_and_b3_multi() {
        let cfg = TraceConfig::default();
        let c = ctx("0102030405060708090a0b0c0d0e0f10", "0102030405060708", true);
        let pairs = inject(&cfg, &c);
        let map: HashMap<String, String> = pairs.into_iter().collect();
        assert!(map.contains_key("traceparent"));
        assert!(map.contains_key("x-b3-traceid"));
    }

    #[test]
    fn inject_includes_tracestate_when_present() {
        let cfg = TraceConfig::default();
        let mut c = ctx("0102030405060708090a0b0c0d0e0f10", "0102030405060708", true);
        c.trace_state = "vendor=foo".into();
        let pairs = inject(&cfg, &c);
        let map: HashMap<String, String> = pairs.into_iter().collect();
        assert_eq!(map.get("tracestate").map(|s| s.as_str()), Some("vendor=foo"));
    }

    #[test]
    fn config_validate_empty_extractors_rejects() {
        let mut cfg = TraceConfig::default();
        cfg.extractors.clear();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn config_validate_invalid_sample_rate_rejects() {
        let mut cfg = TraceConfig::default();
        cfg.sample_rate = -0.1;
        assert!(cfg.validate().is_err());
        cfg.sample_rate = 1.1;
        assert!(cfg.validate().is_err());
    }

    // ── Sampling ──

    #[test]
    fn sample_zero_rate_never() {
        let mut cfg = TraceConfig::default();
        cfg.sample_rate = 0.0;
        assert!(!sample_decision(&cfg, &[0xffu8; 16]));
    }

    #[test]
    fn sample_one_rate_always() {
        let cfg = TraceConfig::default();
        assert!(sample_decision(&cfg, &[0u8; 16]));
        assert!(sample_decision(&cfg, &[0xff; 16]));
    }

    #[test]
    fn sample_rate_deterministic() {
        let mut cfg = TraceConfig::default();
        cfg.sample_rate = 0.5;
        let trace_in = [0u8; 16];
        let trace_out = [0xffu8; 16];
        assert!(sample_decision(&cfg, &trace_in));
        assert!(!sample_decision(&cfg, &trace_out));
    }

    // ── Serde ──

    #[test]
    fn config_json_roundtrip() {
        let cfg = TraceConfig::default();
        let s = serde_json::to_string(&cfg).unwrap();
        let cfg2: TraceConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(cfg, cfg2);
    }

    #[test]
    fn propagator_kebab_serde() {
        assert_eq!(serde_json::to_string(&Propagator::B3Multi).unwrap(), r#""b3-multi""#);
    }
}
