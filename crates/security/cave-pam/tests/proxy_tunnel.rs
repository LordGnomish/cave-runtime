// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for access proxy tunnel state tracking.

use cave_pam::proxy_tunnel::{
    TunnelRegistry, TunnelState, TunnelKind, OpenTunnel,
};
use uuid::Uuid;

fn make_open(kind: TunnelKind, target: &str) -> OpenTunnel {
    OpenTunnel {
        session_id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        node_id: Uuid::new_v4(),
        target_addr: target.to_string(),
        kind,
    }
}

#[test]
fn test_open_tunnel_registers() {
    let reg = TunnelRegistry::new();
    let id = reg.open(make_open(TunnelKind::Ssh, "srv-01:22"));
    let t = reg.get(&id).expect("tunnel should exist");
    assert_eq!(t.state, TunnelState::Active);
    assert_eq!(t.target_addr, "srv-01:22");
}

#[test]
fn test_close_tunnel_changes_state() {
    let reg = TunnelRegistry::new();
    let id = reg.open(make_open(TunnelKind::Database, "db-prod:5432"));
    reg.close(&id).expect("close should succeed");
    let t = reg.get(&id).unwrap();
    assert_eq!(t.state, TunnelState::Closed);
}

#[test]
fn test_list_active_tunnels() {
    let reg = TunnelRegistry::new();
    let id1 = reg.open(make_open(TunnelKind::Ssh, "srv-01:22"));
    let id2 = reg.open(make_open(TunnelKind::Ssh, "srv-02:22"));
    reg.close(&id1).unwrap();
    let active = reg.list_active();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, id2);
}

#[test]
fn test_tunnels_for_user() {
    let reg = TunnelRegistry::new();
    let user_a = Uuid::new_v4();
    let user_b = Uuid::new_v4();
    reg.open(OpenTunnel {
        session_id: Uuid::new_v4(),
        user_id: user_a,
        node_id: Uuid::new_v4(),
        target_addr: "srv-01:22".to_string(),
        kind: TunnelKind::Ssh,
    });
    reg.open(OpenTunnel {
        session_id: Uuid::new_v4(),
        user_id: user_b,
        node_id: Uuid::new_v4(),
        target_addr: "srv-02:22".to_string(),
        kind: TunnelKind::Ssh,
    });
    let a_tunnels = reg.tunnels_for_user(&user_a);
    assert_eq!(a_tunnels.len(), 1);
}

#[test]
fn test_get_nonexistent_returns_none() {
    let reg = TunnelRegistry::new();
    assert!(reg.get(&Uuid::new_v4()).is_none());
}

#[test]
fn test_close_nonexistent_errors() {
    let reg = TunnelRegistry::new();
    assert!(reg.close(&Uuid::new_v4()).is_err());
}

#[test]
fn test_active_count() {
    let reg = TunnelRegistry::new();
    reg.open(make_open(TunnelKind::Kubernetes, "k8s-prod:6443"));
    reg.open(make_open(TunnelKind::Kubernetes, "k8s-staging:6443"));
    assert_eq!(reg.active_count(), 2);
}
