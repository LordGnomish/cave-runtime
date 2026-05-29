// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for multi-tenant index isolation and the Tenant/TenantRegistry aggregate.

use cave_search::tenant::{TenantId, Tenant, TenantRegistry};
use std::str::FromStr;

fn tenant_a() -> TenantId { TenantId::from_str("tenant-a").unwrap() }
fn tenant_b() -> TenantId { TenantId::from_str("tenant-b").unwrap() }

#[test]
fn tenant_id_from_str_valid() {
    let id = TenantId::from_str("my-tenant").unwrap();
    assert_eq!(id.as_str(), "my-tenant");
}

#[test]
fn tenant_id_invalid_uppercase_fails() {
    // DNS-1123: uppercase characters are not allowed.
    assert!(TenantId::from_str("MyTenant").is_err());
}

#[test]
fn tenant_aggregate_stores_id() {
    let t = Tenant::new(tenant_a());
    assert_eq!(t.id.as_str(), "tenant-a");
    assert!(t.display_name.is_none());
}

#[test]
fn tenant_with_display_name() {
    let t = Tenant::with_display_name(tenant_a(), "Tenant Alpha");
    assert_eq!(t.display_name.as_deref(), Some("Tenant Alpha"));
}

#[test]
fn tenant_registry_register_and_get() {
    let mut r = TenantRegistry::new();
    r.register(Tenant::new(tenant_a()));
    assert!(r.get("tenant-a").is_some());
    assert!(r.get("tenant-b").is_none());
}

#[test]
fn tenant_registry_count_after_register() {
    let mut r = TenantRegistry::new();
    assert_eq!(r.count(), 0);
    r.register(Tenant::new(tenant_a()));
    assert_eq!(r.count(), 1);
    r.register(Tenant::new(tenant_b()));
    assert_eq!(r.count(), 2);
}

#[test]
fn tenant_registry_remove() {
    let mut r = TenantRegistry::new();
    r.register(Tenant::new(tenant_a()));
    r.remove("tenant-a");
    assert!(r.get("tenant-a").is_none());
    assert_eq!(r.count(), 0);
}

#[test]
fn tenant_registry_list_ids_sorted() {
    let mut r = TenantRegistry::new();
    r.register(Tenant::new(tenant_b()));
    r.register(Tenant::new(tenant_a()));
    let ids = r.list_ids();
    assert_eq!(ids, vec!["tenant-a", "tenant-b"]);
}

#[test]
fn tenant_index_isolation() {
    use cave_search::index::Index;
    let ta = tenant_a();
    let tb = tenant_b();
    let mut idx_a = Index::new(&ta, "docs");
    let mut idx_b = Index::new(&tb, "docs");
    idx_a.add_document(1, "rust programming");
    idx_b.add_document(1, "python scripting");
    // Each tenant's index is isolated.
    assert!(idx_a.get_doc_ids_for_term("rust").contains(&1));
    assert!(idx_a.get_doc_ids_for_term("python").is_empty());
    assert!(idx_b.get_doc_ids_for_term("python").contains(&1));
    assert!(idx_b.get_doc_ids_for_term("rust").is_empty());
}
