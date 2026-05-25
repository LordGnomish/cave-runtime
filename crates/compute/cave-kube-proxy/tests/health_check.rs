// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Health check NodePort — parity tests against k8s v1.36.0.
//!
//! Upstream: `pkg/proxy/healthcheck/service_health.go`.
//! Per-Service health check NodePorts apply only to Services with
//! externalTrafficPolicy=Local; the server returns 200 iff there is at
//! least one ready local endpoint, otherwise 503.

use cave_kube_proxy::{HealthCheckServer, KubeProxyError, ServicePortName};
use std::collections::HashMap;

const TENANT: &str = "tenant-acme-prod";

fn svc(name: &str) -> ServicePortName {
    ServicePortName::new("default", name, "http")
}

/// Cite: `pkg/proxy/healthcheck/service_health.go:122`
/// (server.SyncServices) — sync replaces the entire tracked-services
/// table; previously-tracked services no longer present are dropped.
#[test]
fn sync_services_replaces_full_table_and_drops_stale() {
    let mut hc = HealthCheckServer::new(TENANT);
    let mut first = HashMap::new();
    first.insert(svc("a"), 32_000);
    first.insert(svc("b"), 32_001);
    hc.sync_services(first);
    assert_eq!(hc.tracked_count(), 2);
    assert_eq!(hc.assigned_port(&svc("a")), Some(32_000));

    let mut second = HashMap::new();
    second.insert(svc("a"), 32_000); // 'b' dropped
    hc.sync_services(second);
    assert_eq!(hc.tracked_count(), 1);
    assert!(hc.assigned_port(&svc("b")).is_none(), "stale entry purged");
}

/// Cite: `pkg/proxy/healthcheck/service_health.go:241`
/// (hcHandler.ServeHTTP) — 200 if local ready endpoints > 0,
/// 503 otherwise.
#[test]
fn http_status_200_when_ready_endpoints_present_503_otherwise() {
    let mut hc = HealthCheckServer::new(TENANT);
    let mut services = HashMap::new();
    services.insert(svc("api"), 32_010);
    hc.sync_services(services);

    let mut counts = HashMap::new();
    counts.insert(svc("api"), 0u32);
    hc.sync_endpoints(counts);
    assert_eq!(hc.http_status(&svc("api")), 503);

    let mut counts = HashMap::new();
    counts.insert(svc("api"), 3u32);
    hc.sync_endpoints(counts);
    assert_eq!(hc.http_status(&svc("api")), 200);
}

/// Cite: `pkg/proxy/healthcheck/service_health.go:241` (404 path) —
/// when the requested Service is not tracked, the handler returns 404.
#[test]
fn http_status_404_for_unknown_service() {
    let hc = HealthCheckServer::new(TENANT);
    assert_eq!(hc.http_status(&svc("does-not-exist")), 404);
}

/// Cite: `pkg/proxy/healthcheck/service_health.go:241` (response body)
/// — body shape includes `localEndpoints` count and Service identity.
/// Cross-tenant access is rejected at the Cave layer with a typed
/// `CrossTenantDenied` error.
#[test]
fn http_body_includes_endpoint_count_and_tenant_guard_works() {
    let mut hc = HealthCheckServer::new(TENANT);
    let mut services = HashMap::new();
    services.insert(svc("api"), 32_020);
    hc.sync_services(services);

    let mut counts = HashMap::new();
    counts.insert(svc("api"), 5u32);
    hc.sync_endpoints(counts);

    let body = hc.http_body(&svc("api"));
    assert_eq!(body["localEndpoints"], 5);
    assert_eq!(body["service"]["namespace"], "default");
    assert_eq!(body["service"]["name"], "api");

    let err = hc.check_tenant("tenant-other").unwrap_err();
    assert!(matches!(err, KubeProxyError::CrossTenantDenied { .. }));
}
