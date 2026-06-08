// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD spec for multi-tenancy security boundaries + tenant resource
//! isolation.
//!
//! Faithful port targets (Kamaji v1.0.0):
//!   internal/utilities/utilities.go     — KamajiLabels / AddTenantPrefix / MergeMaps
//!   internal/constants/labels.go        — label keys/values
//!   api/v1alpha1/datastore_types.go     — DataStoreStatus.UsedBy
//!   internal/resources/datastore/datastore_multitenancy.go — UsedBy gating

use cave_kamaji::isolation::{
    add_tenant_prefix, deregister_usage, kamaji_labels, merge_maps, owns_resource,
    register_usage, tenant_selector, used_by_key,
};
use std::collections::BTreeMap;

// ── Labels (KamajiLabels + constants/labels.go) ─────────────────────────────

#[test]
fn kamaji_labels_carry_project_name_and_component() {
    let l = kamaji_labels("alpha", "datastore-config");
    assert_eq!(l.get("kamaji.clastix.io/project").map(String::as_str), Some("kamaji"));
    assert_eq!(l.get("kamaji.clastix.io/name").map(String::as_str), Some("alpha"));
    assert_eq!(
        l.get("kamaji.clastix.io/component").map(String::as_str),
        Some("datastore-config")
    );
}

// ── AddTenantPrefix ─────────────────────────────────────────────────────────

#[test]
fn tenant_prefix_namespaces_resource_names() {
    // {tcpName}-{name}
    assert_eq!(add_tenant_prefix("datastore-config", "alpha"), "alpha-datastore-config");
}

// ── MergeMaps (override precedence) ─────────────────────────────────────────

#[test]
fn merge_maps_later_wins() {
    let mut a = BTreeMap::new();
    a.insert("k".to_string(), "1".to_string());
    a.insert("x".to_string(), "a".to_string());
    let mut b = BTreeMap::new();
    b.insert("k".to_string(), "2".to_string());
    let out = merge_maps(&[a, b]);
    assert_eq!(out.get("k").map(String::as_str), Some("2")); // later map wins
    assert_eq!(out.get("x").map(String::as_str), Some("a"));
}

// ── Tenant isolation selector + ownership boundary ──────────────────────────

#[test]
fn tenant_selector_scopes_by_control_plane_name() {
    let sel = tenant_selector("alpha");
    assert_eq!(sel.get("kamaji.clastix.io/name").map(String::as_str), Some("alpha"));
}

#[test]
fn ownership_boundary_rejects_cross_tenant_resources() {
    let mut alpha = kamaji_labels("alpha", "deployment");
    assert!(owns_resource(&alpha, "alpha"));
    // a resource labelled for tenant alpha must not be claimable by tenant beta
    assert!(!owns_resource(&alpha, "beta"));
    // a resource with no kamaji ownership label belongs to no tenant
    alpha.remove("kamaji.clastix.io/name");
    assert!(!owns_resource(&alpha, "alpha"));
}

// ── DataStore UsedBy tracking (sharing boundary) ────────────────────────────

#[test]
fn used_by_key_is_namespaced_name() {
    assert_eq!(used_by_key("tenants", "alpha"), "tenants/alpha");
}

#[test]
fn register_usage_is_a_sorted_set() {
    let mut used = vec!["tenants/beta".to_string()];
    assert!(register_usage(&mut used, "tenants/alpha"));
    // idempotent: re-registering the same tenant does not change the set
    assert!(!register_usage(&mut used, "tenants/alpha"));
    assert_eq!(used, vec!["tenants/alpha", "tenants/beta"]); // sorted
}

#[test]
fn deregister_usage_releases_the_datastore() {
    let mut used = vec!["tenants/alpha".to_string(), "tenants/beta".to_string()];
    assert!(deregister_usage(&mut used, "tenants/alpha"));
    assert_eq!(used, vec!["tenants/beta"]);
    // removing an absent owner is a no-op
    assert!(!deregister_usage(&mut used, "tenants/ghost"));
}
