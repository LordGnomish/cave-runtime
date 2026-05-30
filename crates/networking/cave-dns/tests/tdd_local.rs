// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: CoreDNS v1.14.3 `plugin/local/local.go` zone matching.
//!
//! `local` answers queries for `localhost.`, its sub-domains, and the
//! loopback reverse zones with canned replies.

use cave_dns::plugins::local::LocalPlugin;

#[test]
fn matches_localhost() {
    assert!(LocalPlugin::is_local_name("localhost."));
}

#[test]
fn matches_localhost_subdomain() {
    // local.go intercepts `localhost.<domain>` multi-label names.
    assert!(LocalPlugin::is_local_name("foo.localhost."));
}

#[test]
fn matches_loopback_reverse_zones() {
    assert!(LocalPlugin::is_local_name("0.in-addr.arpa."));
    assert!(LocalPlugin::is_local_name("127.in-addr.arpa."));
    assert!(LocalPlugin::is_local_name("255.in-addr.arpa."));
    assert!(LocalPlugin::is_local_name("1.0.0.127.in-addr.arpa."));
}

#[test]
fn ignores_other_names() {
    assert!(!LocalPlugin::is_local_name("example.com."));
    assert!(!LocalPlugin::is_local_name("notlocalhost."));
}
