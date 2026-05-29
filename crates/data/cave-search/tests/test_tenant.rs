// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for multi-tenant index isolation and the Tenant aggregate.

use std::str::FromStr;
use cave_search::tenant::{TenantId, Tenant, TenantRegistry};

fn tenant_a() -> TenantId {
    TenantId::from_str("tenant-a").unwrap()
}

fn tenant_b() -> TenantId {
    TenantId::from_str("tenant-b").unwrap()
}

#[test]
fn tenant_id_from_str_valid() {
    let id = TenantId::from_str("my-tenant").unwrap();
    assert_eq!(id.as_str(), "my-tenant");
}

#[test]
fn tenant_id_invalid_uppercase_fails() {
    // DNS-1123: uppercase not allowed
    assert!(TenantId::from_str("MyTenant").is_err());
}

#[test]
fn tenant_aggregate_stores_id() {
    let id = tenant_a();
    let t = Tenant::new(id.clone());
    assert_eq!(t.id.as_str(), "tenant-a");
}

#[test]
fn tenant_registry_register_and_get() {
    let mut registry = TenantRegistry::new();
    registry.register(Tenant::new(tenant_a()));
    assert!(registry.get("tenant-a").is_some());
    assert!(registry.get("tenant-b").is_none());
}

#[test]
fn tenant_registry_register_multiple() {
    let mut registry = TenantRegistry::new();
    registry.register(Tenant::new(tenant_a()));
    registry.register(Tenant::new(tenant_b()));
    assert_eq!(registry.count(), 2);
}

#[test]
fn tenant_registry_remove() {
    let mut registry = TenantRegistry::new();
    registry.register(Tenant::new(tenant_a()));
    registry.remove("tenant-a");
    assert!(registry.get("tenant-a").is_none());
    assert_eq!(registry.count(), 0);
}

#[test]
fn index_isolation_per_tenant() {
    use cave_search::index::Index;
    let ta = tenant_a();
    let tb = tenant_b();

    let mut idx_a = Index::new(&ta, "docs");
    let mut idx_b = Index::new(&tb, "docs");

    idx_a.add_document(1, "rust programming");
    idx_b.add_document(1, "python scripting");

    // Each index is isolated: tenant-a's doc 1 has "rust", tenant-b's has "python"
    assert!(idx_a.get_doc_ids_for_term("rust").contains(&1));
    assert!(idx_a.get_doc_ids_for_term("python").is_empty());

    assert!(idx_b.get_doc_ids_for_term("python").contains(&1));
    assert!(idx_b.get_doc_ids_for_term("rust").is_empty());
}

#[test]
fn tenant_registry_list_all() {
    let mut registry = TenantRegistry::new();
    registry.register(Tenant::new(tenant_a()));
    registry.register(Tenant::new(tenant_b()));
    let ids = registry.list_ids();
    assert!(ids.contains(&"tenant-a".to_string()));
    assert!(ids.contains(&"tenant-b".to_string()));
}
