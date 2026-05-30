// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: HTTP surface for the EDT bandwidth scheduler and the
//! NAT46/64 address-embedding datapath features.
//!
//! Thin response-builder glue over `ebpf_sim::{edt_sim, nat46x64}` so the
//! portal `/admin/net` page and `cavectl` can probe per-aggregate
//! departure-time pacing and NAT64 address translation. These tests pin
//! the pure JSON shape.

use cave_net::routes::{edt_schedule_json, nat64_translate_json};

#[test]
fn edt_schedule_json_throttle_pushes_departure() {
    // 1 Mbit/s (125000 B/s), t_last 10s, now 9s, 1500-byte packet →
    // admitted, departure stamped 12 ms past t_last.
    let v = edt_schedule_json(125_000, 10_000_000_000, 2_000_000_000, 1500, 9_000_000_000, 9_000_000_000);
    assert_eq!(v["verdict"], "pass");
    assert_eq!(v["tstamp"], 10_012_000_000u64);
    assert_eq!(v["delay_ns"], 12_000_000u64);
    assert_eq!(v["bps"], 125_000u64);
}

#[test]
fn edt_schedule_json_beyond_horizon_drops() {
    // t_last 12s, now 9s → t_next - now = 3.012s >= 2s horizon → drop.
    let v = edt_schedule_json(125_000, 12_000_000_000, 2_000_000_000, 1500, 9_000_000_000, 9_000_000_000);
    assert_eq!(v["verdict"], "drop");
}

#[test]
fn edt_schedule_json_zero_bps_is_pass() {
    let v = edt_schedule_json(0, 0, 2_000_000_000, 1500, 9_000_000_000, 9_000_000_000);
    assert_eq!(v["verdict"], "pass");
}

#[test]
fn nat64_translate_json_round_trips_rfc6052() {
    // Encode 203.0.113.5 in the well-known prefix and recover it.
    let v = nat64_translate_json("203.0.113.5", "rfc6052");
    assert_eq!(v["v4"], "203.0.113.5");
    assert_eq!(v["encoding"], "rfc6052");
    // 64:ff9b:: prefix with the v4 in the low word.
    assert_eq!(v["v6"], "64:ff9b::cb00:7105");
    // round-trip recovery confirms the embedding.
    assert_eq!(v["recovered_v4"], "203.0.113.5");
}

#[test]
fn nat64_translate_json_mapped_encoding() {
    let v = nat64_translate_json("10.0.0.1", "mapped");
    assert_eq!(v["encoding"], "mapped");
    assert_eq!(v["v6"], "::ffff:a00:1");
    assert_eq!(v["recovered_v4"], "10.0.0.1");
}

#[test]
fn nat64_translate_json_bad_ipv4_reports_error() {
    let v = nat64_translate_json("not-an-ip", "mapped");
    assert_eq!(v["error"], "invalid IPv4 address");
}
