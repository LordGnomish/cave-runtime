// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tempo read-path trace combiner — span-level dedup across block/ingester copies.
//!
//! Ports grafana/tempo `pkg/model/trace/combine.go` (`CombineTraceProtos` +
//! `tokenForID`). When Tempo reads a trace it may receive several partial copies
//! of the same trace — one per ingester replica and one per block the trace
//! touched. The combiner merges them into a single trace, deduplicating spans so a
//! span present in multiple copies appears once.
//!
//! Dedup key is a token over the span's ID **and** its kind: a B3-propagated
//! client/server pair legitimately shares one span ID but has two kinds, and both
//! halves must survive. Spans from `a` keep their position; unique spans from `b`
//! are appended in order. The number of `b`-spans dropped as duplicates is
//! returned so callers can account for over-read.
//!
//! Upstream: grafana/tempo (Apache-2.0).

/// A minimal span identity for combination: the raw span-ID bytes plus the OTLP
/// span kind discriminant. Mirrors the (SpanId, Kind) tuple `tokenForID` hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombinerSpan {
    pub span_id: Vec<u8>,
    pub kind: i32,
}

impl CombinerSpan {
    pub fn new(span_id: impl Into<Vec<u8>>, kind: i32) -> Self {
        Self {
            span_id: span_id.into(),
            kind,
        }
    }
}

/// FNV-1 (32-bit) offset basis — matches Go `hash/fnv.New32`.
const FNV_OFFSET_32: u32 = 0x811c9dc5;
/// FNV-1 (32-bit) prime.
const FNV_PRIME_32: u32 = 0x0100_0193;

/// `tokenForID(kind, spanID)` — FNV-1/32 over the span-ID bytes followed by the
/// kind as 4 little-endian bytes (combine.go writes the id then the kind buffer).
pub fn token_for_id(span_id: &[u8], kind: i32) -> u32 {
    let mut h = FNV_OFFSET_32;
    for &b in span_id {
        h = h.wrapping_mul(FNV_PRIME_32);
        h ^= b as u32;
    }
    for &b in &(kind as u32).to_le_bytes() {
        h = h.wrapping_mul(FNV_PRIME_32);
        h ^= b as u32;
    }
    h
}

/// Result of combining two traces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombineResult {
    /// Combined span set (all of `a`, then unique-from-`b`).
    pub spans: Vec<CombinerSpan>,
    /// Number of `b`-spans dropped because they duplicated an `a`-span.
    pub spans_removed: usize,
}

/// `CombineTraceProtos(a, b)` — merge two partial copies of one trace, deduping
/// spans by `(span_id, kind)` token. `a` wins on conflict; unique `b`-spans are
/// appended in their original order.
pub fn combine_traces(a: &[CombinerSpan], b: &[CombinerSpan]) -> CombineResult {
    unimplemented!("RED")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &[u8], kind: i32) -> CombinerSpan {
        CombinerSpan::new(id.to_vec(), kind)
    }

    #[test]
    fn token_is_deterministic_and_kind_sensitive() {
        assert_eq!(token_for_id(b"abc", 2), token_for_id(b"abc", 2));
        // same id, different kind -> different token (client vs server B3 pair)
        assert_ne!(token_for_id(b"abc", 2), token_for_id(b"abc", 3));
        // different id -> different token
        assert_ne!(token_for_id(b"abc", 2), token_for_id(b"abd", 2));
    }

    #[test]
    fn combine_disjoint_keeps_all() {
        let a = vec![s(b"a1", 2), s(b"a2", 2)];
        let b = vec![s(b"b1", 2)];
        let r = combine_traces(&a, &b);
        assert_eq!(r.spans.len(), 3);
        assert_eq!(r.spans_removed, 0);
        // a-order preserved, b appended
        assert_eq!(r.spans[0], s(b"a1", 2));
        assert_eq!(r.spans[1], s(b"a2", 2));
        assert_eq!(r.spans[2], s(b"b1", 2));
    }

    #[test]
    fn combine_dedups_shared_span() {
        let a = vec![s(b"x", 2), s(b"y", 2)];
        let b = vec![s(b"y", 2), s(b"z", 2)]; // y duplicates a
        let r = combine_traces(&a, &b);
        assert_eq!(r.spans_removed, 1);
        assert_eq!(r.spans.len(), 3);
        // only z appended from b
        assert_eq!(r.spans[2], s(b"z", 2));
    }

    #[test]
    fn combine_keeps_same_id_different_kind() {
        // B3 single-header: client + server share span id but differ in kind.
        let a = vec![s(b"shared", 3)]; // client
        let b = vec![s(b"shared", 2)]; // server, same id
        let r = combine_traces(&a, &b);
        assert_eq!(r.spans_removed, 0);
        assert_eq!(r.spans.len(), 2);
    }

    #[test]
    fn combine_empty_b_returns_a() {
        let a = vec![s(b"a1", 2)];
        let r = combine_traces(&a, &[]);
        assert_eq!(r.spans, a);
        assert_eq!(r.spans_removed, 0);
    }

    #[test]
    fn combine_empty_a_returns_b() {
        let b = vec![s(b"b1", 2), s(b"b2", 2)];
        let r = combine_traces(&[], &b);
        assert_eq!(r.spans, b);
        assert_eq!(r.spans_removed, 0);
    }

    #[test]
    fn combine_b_internal_duplicate_of_a_counted_once_each() {
        let a = vec![s(b"p", 2)];
        // b has p twice; both duplicate a's p
        let b = vec![s(b"p", 2), s(b"p", 2), s(b"q", 2)];
        let r = combine_traces(&a, &b);
        assert_eq!(r.spans_removed, 2);
        assert_eq!(r.spans.len(), 2); // p (from a) + q
        assert_eq!(r.spans[1], s(b"q", 2));
    }
}
