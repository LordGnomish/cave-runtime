// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: HTTP surface for the port-range policy datapath feature.
//!
//! Exposes the Cilium PortRangeToMaskedPorts decomposition + the
//! port-range LPM verdict so the portal `/admin/net` page and `cavectl`
//! can render and query L4 policy ranges. The handlers are thin glue
//! over `ebpf_sim::{port_range, policy_lpm}`; these tests pin the pure
//! response-builder shape.

use cave_net::routes::{port_range_decomposition_json, port_range_verdict_json};

#[test]
fn decomposition_json_reports_prefixes_and_count() {
    // 1-1023 → 10 masked-port prefixes (upstream TestPortRange vector).
    let v = port_range_decomposition_json(1, 1023);
    assert_eq!(v["start"], 1);
    assert_eq!(v["end"], 1023);
    assert_eq!(v["prefix_count"], 10);
    let prefixes = v["prefixes"].as_array().expect("prefixes array");
    assert_eq!(prefixes.len(), 10);
    // Each prefix carries port + mask as hex strings and a covered count.
    let first = &prefixes[0];
    assert!(first.get("port").is_some());
    assert!(first.get("mask").is_some());
    assert!(first.get("covered").is_some());
    // The total covered must equal the range width (1023 - 1 + 1 = 1023).
    let total: u64 = prefixes
        .iter()
        .map(|p| p["covered"].as_u64().unwrap())
        .sum();
    assert_eq!(total, 1023);
}

#[test]
fn decomposition_json_full_range_is_single_wildcard() {
    let v = port_range_decomposition_json(0, 65535);
    assert_eq!(v["prefix_count"], 1);
    assert_eq!(v["prefixes"][0]["covered"], 65536);
}

#[test]
fn verdict_json_allows_inside_range_denies_outside() {
    // Allow TCP 8080-8090 for identity 42 ingress; probe 8085 and 9000.
    let inside = port_range_verdict_json(42, 8080, 8090, "TCP", "ingress", 8085);
    assert_eq!(inside["verdict"], "allow");
    assert_eq!(inside["probe_port"], 8085);

    let outside = port_range_verdict_json(42, 8080, 8090, "TCP", "ingress", 9000);
    assert_eq!(outside["verdict"], "deny");
}

#[test]
fn verdict_json_unknown_proto_defaults_to_deny_safely() {
    // An unparseable protocol must not panic; it resolves to deny.
    let v = port_range_verdict_json(42, 80, 80, "SCTP-bogus", "ingress", 80);
    assert_eq!(v["verdict"], "deny");
}
