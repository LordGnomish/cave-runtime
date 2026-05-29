// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for node/resource inventory and enrollment.

use cave_pam::node_inventory::{
    NodeInventory, NodeRecord, NodeKind, NodeHealth, EnrollNode,
};
use uuid::Uuid;
use std::collections::HashMap;

fn make_enroll(kind: NodeKind, hostname: &str) -> EnrollNode {
    EnrollNode {
        hostname: hostname.to_string(),
        kind,
        labels: {
            let mut m = HashMap::new();
            m.insert("env".to_string(), "prod".to_string());
            m
        },
        addr: "10.0.0.1:22".to_string(),
    }
}

#[test]
fn test_enroll_and_get_node() {
    let inv = NodeInventory::new();
    let id = inv.enroll(make_enroll(NodeKind::Server, "web-01")).expect("enroll should succeed");
    let node = inv.get(&id).expect("node should exist");
    assert_eq!(node.hostname, "web-01");
    assert_eq!(node.kind, NodeKind::Server);
    assert_eq!(node.health, NodeHealth::Unknown);
}

#[test]
fn test_list_by_kind() {
    let inv = NodeInventory::new();
    inv.enroll(make_enroll(NodeKind::Server, "srv-01")).unwrap();
    inv.enroll(make_enroll(NodeKind::Server, "srv-02")).unwrap();
    inv.enroll(make_enroll(NodeKind::Database, "db-01")).unwrap();

    let servers = inv.list_by_kind(&NodeKind::Server);
    assert_eq!(servers.len(), 2);
    let dbs = inv.list_by_kind(&NodeKind::Database);
    assert_eq!(dbs.len(), 1);
}

#[test]
fn test_update_health() {
    let inv = NodeInventory::new();
    let id = inv.enroll(make_enroll(NodeKind::Server, "node-01")).unwrap();
    inv.update_health(&id, NodeHealth::Healthy).expect("update should succeed");
    let node = inv.get(&id).unwrap();
    assert_eq!(node.health, NodeHealth::Healthy);
}

#[test]
fn test_deregister_node() {
    let inv = NodeInventory::new();
    let id = inv.enroll(make_enroll(NodeKind::Server, "temp-01")).unwrap();
    inv.deregister(&id).expect("deregister should succeed");
    assert!(inv.get(&id).is_none());
}

#[test]
fn test_list_unhealthy() {
    let inv = NodeInventory::new();
    let id1 = inv.enroll(make_enroll(NodeKind::Server, "sick-01")).unwrap();
    let _id2 = inv.enroll(make_enroll(NodeKind::Server, "healthy-01")).unwrap();
    inv.update_health(&id1, NodeHealth::Unhealthy).unwrap();
    inv.update_health(&_id2, NodeHealth::Healthy).unwrap();

    let unhealthy = inv.list_unhealthy();
    assert_eq!(unhealthy.len(), 1);
    assert_eq!(unhealthy[0].id, id1);
}

#[test]
fn test_label_filter() {
    let inv = NodeInventory::new();
    let mut staging_labels = HashMap::new();
    staging_labels.insert("env".to_string(), "staging".to_string());

    inv.enroll(EnrollNode {
        hostname: "staging-srv".to_string(),
        kind: NodeKind::Server,
        labels: staging_labels,
        addr: "10.0.1.1:22".to_string(),
    }).unwrap();
    inv.enroll(make_enroll(NodeKind::Server, "prod-srv")).unwrap(); // has env=prod

    let prod_nodes = inv.list_by_label("env", "prod");
    assert_eq!(prod_nodes.len(), 1);
    assert_eq!(prod_nodes[0].hostname, "prod-srv");
}

#[test]
fn test_get_nonexistent_returns_none() {
    let inv = NodeInventory::new();
    assert!(inv.get(&Uuid::new_v4()).is_none());
}
