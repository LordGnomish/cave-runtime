// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// RED→GREEN cycle 7 (continuation ray #3): port of
// pkg/apis/v1/duration.go from kubernetes-sigs/karpenter v1.12.1 (sha
// ed490e8) — NillableDuration with the "Never" sentinel, plus a faithful port
// of Go's stdlib time.ParseDuration / Duration.String() that it delegates to.

use cave_karpenter::duration::{format_duration, parse_duration, NillableDuration, NEVER};

const NS: i64 = 1;
const US: i64 = 1_000;
const MS: i64 = 1_000_000;
const S: i64 = 1_000_000_000;
const M: i64 = 60 * S;
const H: i64 = 60 * M;

// ── parse_duration (Go time.ParseDuration) ───────────────────────────────────

#[test]
fn parse_duration_simple_units() {
    assert_eq!(parse_duration("300ms").unwrap(), 300 * MS);
    assert_eq!(parse_duration("1ns").unwrap(), NS);
    assert_eq!(parse_duration("1us").unwrap(), US);
    assert_eq!(parse_duration("1µs").unwrap(), US); // U+00B5 micro sign
    assert_eq!(parse_duration("1s").unwrap(), S);
    assert_eq!(parse_duration("1m").unwrap(), M);
    assert_eq!(parse_duration("1h").unwrap(), H);
}

#[test]
fn parse_duration_zero_and_compound() {
    assert_eq!(parse_duration("0").unwrap(), 0);
    assert_eq!(parse_duration("2h45m").unwrap(), 2 * H + 45 * M);
    assert_eq!(parse_duration("1h30m0s").unwrap(), H + 30 * M);
}

#[test]
fn parse_duration_fractional() {
    assert_eq!(parse_duration("1.5h").unwrap(), H + 30 * M);
    assert_eq!(parse_duration("1.5s").unwrap(), S + 500 * MS);
    assert_eq!(parse_duration("0.5s").unwrap(), 500 * MS);
}

#[test]
fn parse_duration_signed() {
    assert_eq!(parse_duration("-1.5h").unwrap(), -(H + 30 * M));
    assert_eq!(parse_duration("+2h").unwrap(), 2 * H);
}

#[test]
fn parse_duration_rejects_bad_input() {
    assert!(parse_duration("").is_err());
    assert!(parse_duration("abc").is_err());
    assert!(parse_duration("10").is_err()); // missing unit
    assert!(parse_duration("1x").is_err()); // unknown unit
    assert!(parse_duration(".s").is_err()); // no digits
}

// ── format_duration (Go Duration.String) ─────────────────────────────────────

#[test]
fn format_duration_sub_second() {
    assert_eq!(format_duration(0), "0s");
    assert_eq!(format_duration(NS), "1ns");
    assert_eq!(format_duration(300 * MS), "300ms");
    assert_eq!(format_duration(US), "1µs");
}

#[test]
fn format_duration_compound() {
    assert_eq!(format_duration(S), "1s");
    assert_eq!(format_duration(S + 500 * MS), "1.5s");
    assert_eq!(format_duration(H + 30 * M), "1h30m0s");
    assert_eq!(format_duration(M), "1m0s");
}

#[test]
fn format_duration_negative() {
    assert_eq!(format_duration(-H), "-1h0m0s");
    assert_eq!(format_duration(-300 * MS), "-300ms");
}

#[test]
fn format_then_parse_roundtrips() {
    for v in [S, H + 30 * M, 300 * MS, 2 * H + 45 * M, US, NS] {
        let s = format_duration(v);
        assert_eq!(parse_duration(&s).unwrap(), v, "roundtrip failed for {v}");
    }
}

// ── NillableDuration ─────────────────────────────────────────────────────────

#[test]
fn nillable_never_is_disabled() {
    let nd = NillableDuration::never();
    assert!(nd.is_never());
    assert_eq!(nd.nanos(), None);
}

#[test]
fn nillable_parse_never_sentinel() {
    let nd = NillableDuration::parse(NEVER).unwrap();
    assert!(nd.is_never());
}

#[test]
fn nillable_parse_duration_value() {
    let nd = NillableDuration::parse("30m").unwrap();
    assert_eq!(nd.nanos(), Some(30 * M));
    assert!(!nd.is_never());
}

#[test]
fn nillable_serde_roundtrip_value_preserves_raw() {
    // Raw is preserved on (de)serialize to avoid drift (30m must not become 30m0s)
    let nd = NillableDuration::parse("30m").unwrap();
    let json = serde_json::to_string(&nd).unwrap();
    assert_eq!(json, "\"30m\"");
    let back: NillableDuration = serde_json::from_str(&json).unwrap();
    assert_eq!(back.nanos(), Some(30 * M));
}

#[test]
fn nillable_serde_roundtrip_never() {
    let nd = NillableDuration::never();
    let json = serde_json::to_string(&nd).unwrap();
    assert_eq!(json, "\"Never\"");
    let back: NillableDuration = serde_json::from_str(&json).unwrap();
    assert!(back.is_never());
}

#[test]
fn nillable_marshal_from_nanos_uses_string_form() {
    // A NillableDuration built from nanos (no Raw) marshals via Duration.String()
    let nd = NillableDuration::from_nanos(H + 30 * M);
    let json = serde_json::to_string(&nd).unwrap();
    assert_eq!(json, "\"1h30m0s\"");
}
