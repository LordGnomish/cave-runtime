// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Offset store tests — checkpoint persistence for connector resume.
//!
//! Cite: debezium-storage `io.debezium.storage.OffsetBackingStore` +
//! `InMemoryOffsetBackingStore` + the KV interface that every
//! connector uses to persist / recover its streaming offset.

use cave_cdc::offset::{OffsetKey, OffsetStore};

const TENANT: &str = "acme-offset-test";

#[test]
fn offset_store_set_then_get_round_trips() {
    let mut store = OffsetStore::new(TENANT);
    let key = OffsetKey::new(TENANT, "pg-connector", "public", "orders");
    store.set(key.clone(), serde_json::json!({"lsn": "0/15D3C60"}));
    let val = store.get(&key).unwrap();
    assert_eq!(val["lsn"], "0/15D3C60");
}

#[test]
fn offset_store_get_missing_returns_none() {
    let store = OffsetStore::new(TENANT);
    let key = OffsetKey::new(TENANT, "pg-connector", "public", "orders");
    assert!(store.get(&key).is_none());
}

#[test]
fn offset_store_overwrite_updates_value() {
    let mut store = OffsetStore::new(TENANT);
    let key = OffsetKey::new(TENANT, "pg-connector", "public", "orders");
    store.set(key.clone(), serde_json::json!({"lsn": "0/100"}));
    store.set(key.clone(), serde_json::json!({"lsn": "0/200"}));
    let val = store.get(&key).unwrap();
    assert_eq!(val["lsn"], "0/200");
}

#[test]
fn offset_store_delete_removes_entry() {
    let mut store = OffsetStore::new(TENANT);
    let key = OffsetKey::new(TENANT, "pg-connector", "public", "orders");
    store.set(key.clone(), serde_json::json!({"lsn": "0/100"}));
    store.delete(&key);
    assert!(store.get(&key).is_none());
}

#[test]
fn offset_store_rejects_cross_tenant_set() {
    let mut store = OffsetStore::new(TENANT);
    let key = OffsetKey::new("other-tenant", "pg-connector", "public", "orders");
    let err = store.set_checked(key, serde_json::json!({"lsn": "0/100"})).unwrap_err();
    assert!(err.to_string().contains("cross-tenant"), "should be cross-tenant error: {}", err);
}

#[test]
fn offset_key_serializes_to_stable_string() {
    let key = OffsetKey::new(TENANT, "pg-connector", "public", "orders");
    let s = key.to_key_string();
    // Key must embed tenant_id so different tenants never collide.
    assert!(s.contains(TENANT), "key should contain tenant: {}", s);
    assert!(s.contains("pg-connector"), "key should contain connector: {}", s);
}

#[test]
fn offset_store_all_offsets_returns_snapshot() {
    let mut store = OffsetStore::new(TENANT);
    let k1 = OffsetKey::new(TENANT, "pg-connector", "public", "orders");
    let k2 = OffsetKey::new(TENANT, "pg-connector", "public", "items");
    store.set(k1, serde_json::json!({"lsn": "0/100"}));
    store.set(k2, serde_json::json!({"lsn": "0/200"}));
    let snap = store.all_offsets();
    assert_eq!(snap.len(), 2);
}
