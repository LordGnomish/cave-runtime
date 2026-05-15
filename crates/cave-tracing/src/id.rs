// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Trace/span ID generation. Uses a thread-local PRNG seeded from
//! system time + thread id; non-cryptographic but well-distributed
//! enough for SDK ID assignment per OpenTelemetry guidance.

use crate::types::{SpanId, TraceId};
use std::cell::Cell;

thread_local! {
    static STATE: Cell<u128> = Cell::new(seed());
}

fn seed() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = std::thread::current().id();
    let mix = format!("{:?}-{}", id, nanos);
    let mut h: u128 = 0xcbf29ce4_84222325_cbf29ce4_84222325;
    for b in mix.as_bytes() {
        h ^= *b as u128;
        h = h.wrapping_mul(0x00000100_000001b3);
    }
    if h == 0 { 1 } else { h }
}

fn next() -> u128 {
    STATE.with(|s| {
        let mut x = s.get();
        // xorshift64 stretched to 128 by XOR-mixing two halves
        let mut hi = (x >> 64) as u64;
        let mut lo = x as u64;
        hi ^= hi << 13; hi ^= hi >> 7; hi ^= hi << 17;
        lo ^= lo << 15; lo ^= lo >> 19; lo ^= lo << 23;
        let next = ((hi as u128) << 64) | (lo as u128);
        x = if next == 0 { 1 } else { next };
        s.set(x);
        x
    })
}

pub fn new_trace_id() -> TraceId {
    let v = next();
    if v == 0 { 1 } else { v }
}

pub fn new_span_id() -> SpanId {
    let v = next() as u64;
    if v == 0 { 1 } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_trace_ids_are_nonzero() {
        for _ in 0..200 { assert_ne!(new_trace_id(), 0); }
    }

    #[test]
    fn test_span_ids_are_nonzero() {
        for _ in 0..200 { assert_ne!(new_span_id(), 0); }
    }

    #[test]
    fn test_trace_ids_have_low_collision_rate() {
        let mut seen = HashSet::new();
        for _ in 0..10_000 {
            let id = new_trace_id();
            assert!(seen.insert(id), "collision on {}", id);
        }
    }

    #[test]
    fn test_span_ids_have_low_collision_rate() {
        let mut seen = HashSet::new();
        let mut collisions = 0;
        for _ in 0..50_000 {
            if !seen.insert(new_span_id()) {
                collisions += 1;
            }
        }
        // 50k draws from u64 space — expected collisions ~0
        assert!(collisions < 5, "{} collisions in 50k draws", collisions);
    }
}
