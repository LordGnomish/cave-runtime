// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: CoreDNS v1.14.3 `plugin/minimal/minimal.go` behaviour.
//!
//! `minimal` strips the authority + additional sections from positive
//! (NOERROR-with-answers) responses, but passes denial / error / delegation
//! responses through untouched.

use cave_dns::plugins::minimal::MinimalPlugin;
use hickory_proto::op::ResponseCode;

#[test]
fn minimizes_positive_noerror_with_answers() {
    assert!(MinimalPlugin::should_minimize(ResponseCode::NoError, true));
}

#[test]
fn passes_through_nxdomain() {
    // response.Denial class — not minimized.
    assert!(!MinimalPlugin::should_minimize(ResponseCode::NXDomain, false));
}

#[test]
fn passes_through_nodata() {
    // NOERROR with no answers (NODATA / delegation) — not minimized.
    assert!(!MinimalPlugin::should_minimize(ResponseCode::NoError, false));
}

#[test]
fn passes_through_servfail() {
    // response.Error class — not minimized.
    assert!(!MinimalPlugin::should_minimize(ResponseCode::ServFail, false));
}
