// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ServiceChangeTracker — parity tests against k8s v1.36.0.
//!
//! Upstream: `pkg/proxy/config/config.go` (ServiceConfig event handlers)
//! and the implicit change-tracking the proxier maintains via
//! `serviceChanges` in `pkg/proxy/iptables/proxier.go` and
//! `pkg/proxy/nftables/proxier.go`. Multi-tenant: every test uses an
//! explicit `tenant_id`.

use cave_kube_proxy::{
    KubeProxyError, Protocol, ServiceChangeTracker, ServicePortInfo, ServicePortName,
};
use std::net::IpAddr;

const TENANT: &str = "tenant-acme-prod";

fn svc(name: &str, ip: &str, port: u16) -> ServicePortInfo {
    ServicePortInfo::cluster_ip_only(
        TENANT,
        ServicePortName::new("default", name, "http"),
        ip.parse::<IpAddr>().unwrap(),
        port,
        Protocol::Tcp,
    )
}

/// Cite: `pkg/proxy/config/config.go:212` (ServiceConfig.handleAddService)
/// — an Add (previous=None, current=Some) increments pending by 1 and
/// records the change as `is_add`.
#[test]
fn add_event_records_one_pending_change() {
    let mut t = ServiceChangeTracker::new(TENANT);
    let s = svc("api", "10.0.0.1", 80);
    t.update(None, Some(s.clone())).unwrap();
    assert_eq!(t.pending_count(), 1);
    let change = t.pending_for(&s.name).unwrap();
    assert!(change.is_add());
    assert!(!change.is_update());
}

/// Cite: `pkg/proxy/config/config.go:224` (handleUpdateService) — an
/// Update on a pending Add must coalesce: the resulting change keeps
/// `previous = None` (still net-add) but uses the latest `current`.
#[test]
fn add_then_update_coalesces_to_single_pending_change() {
    let mut t = ServiceChangeTracker::new(TENANT);
    let s_v1 = svc("api", "10.0.0.1", 80);
    let s_v2 = ServicePortInfo { port: 8080, ..s_v1.clone() };
    t.update(None, Some(s_v1.clone())).unwrap();
    t.update(Some(s_v1.clone()), Some(s_v2.clone())).unwrap();

    assert_eq!(t.pending_count(), 1, "Add+Update must coalesce");
    let change = t.pending_for(&s_v1.name).unwrap();
    assert!(change.is_add(), "net effect is still an add");
    assert_eq!(change.current.as_ref().unwrap().port, 8080);
}

/// Cite: `pkg/proxy/config/config.go:241` (handleDeleteService) +
/// upstream coalescing: an Add followed by a Delete cancels out and the
/// pending entry is dropped entirely.
#[test]
fn add_then_delete_cancels_pending_change() {
    let mut t = ServiceChangeTracker::new(TENANT);
    let s = svc("api", "10.0.0.1", 80);
    t.update(None, Some(s.clone())).unwrap();
    t.update(Some(s.clone()), None).unwrap();
    assert_eq!(t.pending_count(), 0, "Add+Delete is a no-op");
}

/// Cite: cave multi-tenancy invariant — the tracker rejects writes
/// from a different tenant_id with a `CrossTenantDenied` error rather
/// than silently leaking.
#[test]
fn cross_tenant_update_is_rejected() {
    let mut t = ServiceChangeTracker::new(TENANT);
    let foreign = ServicePortInfo {
        tenant_id: "tenant-other".into(),
        ..svc("api", "10.0.0.1", 80)
    };
    let err = t.update(None, Some(foreign)).unwrap_err();
    assert!(matches!(err, KubeProxyError::CrossTenantDenied { .. }));
    assert_eq!(t.pending_count(), 0);
}
