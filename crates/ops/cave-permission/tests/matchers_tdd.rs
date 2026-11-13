// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD coverage for Casbin built-in matcher operators.
//!
//! Upstream (Apache-2.0, line-port permitted): casbin v3.10.0
//!   util/builtin_operators.go — KeyMatch / KeyMatch2 / KeyMatch3 / RegexMatch / IPMatch
//!
//! These pure matcher functions are the foundation of Casbin's policy matchers
//! (e.g. `m = keyMatch2(r.obj, p.obj) && r.act == p.act`). The crate previously
//! had no matcher operators at all (audited MISSING), so authorization could not
//! express wildcard/path/CIDR rules.

use cave_permission::matchers::{ip_match, key_match, key_match2, key_match3, regex_match};

#[test]
fn key_match_wildcard_matches_path() {
    // `*` matches any suffix from its position.
    assert!(key_match("/foo/bar", "/foo/*"));
    assert!(key_match("/foo/bar/baz", "/foo/*"));
    assert!(!key_match("/foo/bar", "/baz/*"));
    // No wildcard => exact equality.
    assert!(key_match("/foo", "/foo"));
    assert!(!key_match("/foo", "/bar"));
}

#[test]
fn key_match2_named_param_matches_single_segment() {
    // `:id` matches exactly one path segment ([^/]+), anchored.
    assert!(key_match2("/foo/bar", "/foo/:id"));
    assert!(!key_match2("/foo/bar/baz", "/foo/:id")); // :id is single-segment
    assert!(key_match2("/foo/bar/baz", "/foo/:id/baz"));
    // `/*` still expands to `/.*`.
    assert!(key_match2("/foo/bar/baz", "/foo/*"));
}

#[test]
fn key_match3_brace_param_matches_single_segment() {
    assert!(key_match3("/foo/bar", "/foo/{id}"));
    assert!(!key_match3("/foo/bar/baz", "/foo/{id}"));
    assert!(key_match3("/foo/bar/baz", "/foo/{id}/baz"));
}

#[test]
fn regex_match_anchors_as_written() {
    assert!(regex_match("foobar", "^foo.*"));
    assert!(!regex_match("barfoo", "^foo.*"));
    assert!(regex_match("/topic/1", "/topic/[0-9]+"));
}

#[test]
fn ip_match_cidr_and_exact() {
    // CIDR containment.
    assert!(ip_match("192.168.2.123", "192.168.2.0/24"));
    assert!(!ip_match("192.168.3.1", "192.168.2.0/24"));
    // Bare IP => exact equality.
    assert!(ip_match("127.0.0.1", "127.0.0.1"));
    assert!(!ip_match("127.0.0.2", "127.0.0.1"));
    // IPv6 CIDR.
    assert!(ip_match("2001:db8::1", "2001:db8::/32"));
    // Invalid inputs are non-matches, not panics.
    assert!(!ip_match("not-an-ip", "192.168.2.0/24"));
}
