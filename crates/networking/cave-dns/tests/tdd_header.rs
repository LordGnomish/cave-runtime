// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: CoreDNS v1.14.3 `plugin/header/header.go` rule parsing.
//!
//! `header` sets/clears the `aa`, `ra`, `rd` flags on queries and responses.

use cave_dns::plugins::header::HeaderPlugin;

#[test]
fn parse_set_authoritative() {
    let rules = HeaderPlugin::parse_rules("set", &["aa"]).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].flag, "aa");
    assert!(rules[0].state);
}

#[test]
fn parse_clear_multiple() {
    let rules = HeaderPlugin::parse_rules("clear", &["ra", "rd"]).unwrap();
    assert_eq!(rules.len(), 2);
    assert!(!rules[0].state);
    assert!(!rules[1].state);
}

#[test]
fn rejects_unknown_flag() {
    // header.go newRules(): unknown/unsupported flag is an error.
    assert!(HeaderPlugin::parse_rules("set", &["zz"]).is_err());
}

#[test]
fn rejects_unknown_action() {
    // header.go newRules(): action must be set or clear.
    assert!(HeaderPlugin::parse_rules("frob", &["aa"]).is_err());
}

#[test]
fn rejects_empty_flag_list() {
    // header.go newRules(): at least one flag must be provided.
    assert!(HeaderPlugin::parse_rules("set", &[]).is_err());
}
