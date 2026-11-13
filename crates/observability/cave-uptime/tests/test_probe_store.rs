// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Test: probe store CRUD operations

use cave_uptime::models::{ProbeType, UptimeProbe};
use cave_uptime::store::ProbeStore;
use uuid::Uuid;

fn make_probe(name: &str, probe_type: ProbeType) -> UptimeProbe {
    UptimeProbe {
        id: Uuid::new_v4(),
        name: name.to_string(),
        target_url: "https://example.com".to_string(),
        probe_type,
        interval_seconds: 60,
        timeout_ms: 5000,
        enabled: true,
    }
}

#[test]
fn test_store_insert_and_get() {
    let store = ProbeStore::new();
    let probe = make_probe("test-probe", ProbeType::Http);
    let id = probe.id;
    store.insert(probe.clone());
    let retrieved = store.get(id).expect("probe should exist");
    assert_eq!(retrieved.id, id);
    assert_eq!(retrieved.name, "test-probe");
}

#[test]
fn test_store_list_all() {
    let store = ProbeStore::new();
    store.insert(make_probe("a", ProbeType::Http));
    store.insert(make_probe("b", ProbeType::Tcp));
    store.insert(make_probe("c", ProbeType::Dns));
    let all = store.list();
    assert_eq!(all.len(), 3);
}

#[test]
fn test_store_update() {
    let store = ProbeStore::new();
    let mut probe = make_probe("original", ProbeType::Http);
    let id = probe.id;
    store.insert(probe.clone());
    probe.name = "updated".to_string();
    probe.enabled = false;
    let ok = store.update(probe);
    assert!(ok);
    let retrieved = store.get(id).unwrap();
    assert_eq!(retrieved.name, "updated");
    assert!(!retrieved.enabled);
}

#[test]
fn test_store_delete() {
    let store = ProbeStore::new();
    let probe = make_probe("delete-me", ProbeType::Http);
    let id = probe.id;
    store.insert(probe);
    assert!(store.delete(id));
    assert!(store.get(id).is_none());
    // deleting non-existent returns false
    assert!(!store.delete(id));
}

#[test]
fn test_store_update_nonexistent_returns_false() {
    let store = ProbeStore::new();
    let probe = make_probe("ghost", ProbeType::Http);
    assert!(!store.update(probe));
}

#[test]
fn test_store_empty_list() {
    let store = ProbeStore::new();
    assert!(store.list().is_empty());
}
