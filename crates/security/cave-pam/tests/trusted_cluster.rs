// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for trusted cluster federation model.

use cave_pam::trusted_cluster::{
    TrustStore, TrustedCluster, TrustDirection, TrustState,
};
use uuid::Uuid;

fn make_cluster(name: &str, direction: TrustDirection) -> TrustedCluster {
    TrustedCluster {
        name: name.to_string(),
        root_ca_pem: format!("-----BEGIN CERTIFICATE-----\nFAKE-CA-{name}\n-----END CERTIFICATE-----"),
        direction,
        roles_to_map: vec!["admin".to_string()],
        metadata: std::collections::HashMap::new(),
    }
}

#[test]
fn test_register_cluster() {
    let store = TrustStore::new("cluster-a");
    let cluster = make_cluster("cluster-b", TrustDirection::Bidirectional);
    let id = store.register(cluster).expect("should register");
    let fetched = store.get(&id).expect("should find cluster");
    assert_eq!(fetched.name, "cluster-b");
    assert_eq!(fetched.state, TrustState::Pending);
}

#[test]
fn test_activate_cluster() {
    let store = TrustStore::new("cluster-a");
    let id = store.register(make_cluster("cluster-b", TrustDirection::Outbound)).unwrap();
    store.activate(&id).expect("should activate");
    let c = store.get(&id).unwrap();
    assert_eq!(c.state, TrustState::Active);
}

#[test]
fn test_deactivate_cluster() {
    let store = TrustStore::new("cluster-a");
    let id = store.register(make_cluster("cluster-c", TrustDirection::Inbound)).unwrap();
    store.activate(&id).unwrap();
    store.deactivate(&id).expect("should deactivate");
    let c = store.get(&id).unwrap();
    assert_eq!(c.state, TrustState::Inactive);
}

#[test]
fn test_remove_cluster() {
    let store = TrustStore::new("cluster-a");
    let id = store.register(make_cluster("cluster-d", TrustDirection::Bidirectional)).unwrap();
    store.remove(&id).expect("should remove");
    assert!(store.get(&id).is_none());
}

#[test]
fn test_list_active_clusters() {
    let store = TrustStore::new("cluster-a");
    let id1 = store.register(make_cluster("c1", TrustDirection::Outbound)).unwrap();
    let _id2 = store.register(make_cluster("c2", TrustDirection::Inbound)).unwrap();
    store.activate(&id1).unwrap();

    let active = store.list_active();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "c1");
}

#[test]
fn test_get_nonexistent_returns_none() {
    let store = TrustStore::new("cluster-a");
    assert!(store.get(&Uuid::new_v4()).is_none());
}
