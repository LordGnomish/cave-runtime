// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for Grafana Mimir multi-tenant `X-Scope-OrgID` handling.
//!
//! Upstream: grafana/mimir `pkg/tenant/tenant.go` — `TenantIDs`,
//! `NormalizeTenantIDs`, `ValidTenantID`. A single `X-Scope-OrgID` header may
//! carry several tenants separated by `|`; the parsed list is validated,
//! de-duplicated and sorted. Tenant IDs are bounded to 150 bytes, restricted
//! to a safe character set, and must not be the path-unsafe `.` or `..`.

use cave_metrics::multitenant::{parse_tenant_ids, valid_tenant_id, MAX_TENANT_ID_LENGTH};

#[test]
fn single_tenant_parses() {
    assert_eq!(parse_tenant_ids("acme").unwrap(), vec!["acme".to_string()]);
}

#[test]
fn multi_tenant_is_sorted_and_deduped() {
    assert_eq!(
        parse_tenant_ids("tenant2|tenant1|tenant2").unwrap(),
        vec!["tenant1".to_string(), "tenant2".to_string()]
    );
}

#[test]
fn surrounding_whitespace_trimmed() {
    assert_eq!(
        parse_tenant_ids(" a | b ").unwrap(),
        vec!["a".to_string(), "b".to_string()]
    );
}

#[test]
fn empty_header_is_rejected() {
    assert!(parse_tenant_ids("").is_err());
    assert!(parse_tenant_ids("   ").is_err());
}

#[test]
fn unsafe_path_segments_rejected() {
    assert!(valid_tenant_id(".").is_err());
    assert!(valid_tenant_id("..").is_err());
    assert!(parse_tenant_ids("good|..").is_err());
}

#[test]
fn unsupported_characters_rejected() {
    assert!(valid_tenant_id("a/b").is_err());
    assert!(valid_tenant_id("a\\b").is_err());
    assert!(valid_tenant_id("a b").is_err());
    assert!(valid_tenant_id("a#b").is_err());
}

#[test]
fn supported_characters_accepted() {
    // Mimir's safe set: alphanumerics plus  ! - _ . * ' ( )
    assert!(valid_tenant_id("team-1_prod.v2").is_ok());
    assert!(valid_tenant_id("a!*'()").is_ok());
}

#[test]
fn over_length_rejected() {
    let long = "a".repeat(MAX_TENANT_ID_LENGTH + 1);
    assert!(valid_tenant_id(&long).is_err());
    let at_limit = "a".repeat(MAX_TENANT_ID_LENGTH);
    assert!(valid_tenant_id(&at_limit).is_ok());
}
