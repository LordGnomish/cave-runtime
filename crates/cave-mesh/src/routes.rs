// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Admin API — full CRUD for all mesh resources + xDS snapshot + multi-cluster.
//!
//! Base path: /api/mesh/
//!
//!   Core mesh resources  (VS, DR, GW, SE, PA, RA, AP, RL, CB)
//!   Sidecar              /api/mesh/sidecars
//!   EnvoyFilter          /api/mesh/envoyfilters
//!   WorkloadGroup        /api/mesh/workloadgroups
//!   WorkloadEntry        /api/mesh/workloadentries
//!   Telemetry            /api/mesh/telemetries
//!   xDS                  /api/mesh/xds/snapshot, /api/mesh/xds/nodes
//!   Multi-cluster        /api/mesh/multicluster/clusters, /federations
//!   Observability        /api/mesh/obs/metrics/{id}, /golden/{id}
//!   Health / Metrics     /api/mesh/health, /api/mesh/metrics

use crate::{
    models::{
        AuthorizationPolicy, DestinationRule, EnvoyFilter, Gateway, HealthStatus,
        PeerAuthentication, RateLimitPolicy, RequestAuthentication, ServiceEntry, ServiceMeta,
        Sidecar, Telemetry, VirtualService, WorkloadEntry, WorkloadGroup,
    },
    multicluster::{RemoteCluster, TrustDomainFederation},
    xds::{NodeInfo, XdsSnapshot},
    MeshState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<MeshState>) -> Router {
    Router::new()
        // ── Health / Prometheus metrics ──────────────────────
        .route("/api/mesh/health", get(health))
        .route("/api/mesh/metrics", get(metrics_handler))
        // ── Service Registry ─────────────────────────────────
        .route("/api/mesh/services", get(list_services).post(register_service))
        .route("/api/mesh/services/{ns}/{name}", get(get_service).delete(deregister_service))
        .route("/api/mesh/services/{ns}/{name}/health/{addr}/{port}", put(update_health))
        // ── VirtualServices ──────────────────────────────────
        .route(
            "/api/mesh/virtualservices",
            get(list_virtual_services).post(upsert_virtual_service),
        )
        .route(
            "/api/mesh/virtualservices/{host}",
            get(get_virtual_service).delete(delete_virtual_service),
        )
        // ── DestinationRules ─────────────────────────────────
        .route(
            "/api/mesh/destinationrules",
            get(list_destination_rules).post(upsert_destination_rule),
        )
        .route(
            "/api/mesh/destinationrules/{host}",
            get(get_destination_rule).delete(delete_destination_rule),
        )
        // ── Gateways ─────────────────────────────────────────
        .route("/api/mesh/gateways", get(list_gateways).post(upsert_gateway))
        .route("/api/mesh/gateways/{ns}/{name}", get(get_gateway).delete(delete_gateway))
        // ── ServiceEntries ───────────────────────────────────
        .route(
            "/api/mesh/serviceentries",
            get(list_service_entries).post(upsert_service_entry),
        )
        .route(
            "/api/mesh/serviceentries/{ns}/{name}",
            get(get_service_entry).delete(delete_service_entry),
        )
        // ── PeerAuthentication ───────────────────────────────
        .route(
            "/api/mesh/peerauthentications",
            get(list_peer_auth).post(upsert_peer_auth),
        )
        .route(
            "/api/mesh/peerauthentications/{ns}/{name}",
            get(get_peer_auth).delete(delete_peer_auth),
        )
        // ── RequestAuthentication ────────────────────────────
        .route(
            "/api/mesh/requestauthentications",
            get(list_request_auth).post(upsert_request_auth),
        )
        .route(
            "/api/mesh/requestauthentications/{ns}/{name}",
            delete(delete_request_auth),
        )
        // ── AuthorizationPolicy ──────────────────────────────
        .route(
            "/api/mesh/authorizationpolicies",
            get(list_authz_policies).post(upsert_authz_policy),
        )
        .route(
            "/api/mesh/authorizationpolicies/{ns}/{name}",
            delete(delete_authz_policy),
        )
        // ── RateLimit ────────────────────────────────────────
        .route("/api/mesh/ratelimits", get(list_rate_limits).post(upsert_rate_limit))
        .route("/api/mesh/ratelimits/{name}", delete(delete_rate_limit))
        // ── Circuit Breakers ─────────────────────────────────
        .route("/api/mesh/circuitbreakers", get(list_circuit_breakers))
        // ── Sidecar ──────────────────────────────────────────
        .route("/api/mesh/sidecars", get(list_sidecars).post(upsert_sidecar))
        .route("/api/mesh/sidecars/{ns}/{name}", get(get_sidecar).delete(delete_sidecar))
        // ── EnvoyFilter ──────────────────────────────────────
        .route("/api/mesh/envoyfilters", get(list_envoy_filters).post(upsert_envoy_filter))
        .route(
            "/api/mesh/envoyfilters/{ns}/{name}",
            get(get_envoy_filter).delete(delete_envoy_filter),
        )
        // ── WorkloadGroup ────────────────────────────────────
        .route(
            "/api/mesh/workloadgroups",
            get(list_workload_groups).post(upsert_workload_group),
        )
        .route(
            "/api/mesh/workloadgroups/{ns}/{name}",
            get(get_workload_group).delete(delete_workload_group),
        )
        // ── WorkloadEntry ────────────────────────────────────
        .route(
            "/api/mesh/workloadentries",
            get(list_workload_entries).post(upsert_workload_entry),
        )
        .route(
            "/api/mesh/workloadentries/{ns}/{name}",
            delete(delete_workload_entry),
        )
        // ── Telemetry ────────────────────────────────────────
        .route("/api/mesh/telemetries", get(list_telemetries).post(upsert_telemetry))
        .route(
            "/api/mesh/telemetries/{ns}/{name}",
            get(get_telemetry).delete(delete_telemetry),
        )
        // ── xDS ──────────────────────────────────────────────
        .route("/api/mesh/xds/snapshot", get(get_xds_snapshot).post(set_xds_snapshot))
        .route("/api/mesh/xds/nodes", get(list_xds_nodes))
        .route("/api/mesh/xds/status", get(xds_sync_status))
        // ── Multi-cluster ────────────────────────────────────
        .route(
            "/api/mesh/multicluster/clusters",
            get(list_clusters).post(register_cluster),
        )
        .route(
            "/api/mesh/multicluster/clusters/{name}",
            get(get_cluster).delete(remove_cluster),
        )
        .route(
            "/api/mesh/multicluster/federations",
            get(list_federations).post(add_federation),
        )
        .route("/api/mesh/multicluster/status", get(multicluster_status))
        // ── Observability (golden signals) ───────────────────
        .route("/api/mesh/obs/metrics/{id}", get(obs_metrics))
        .route("/api/mesh/obs/golden/{id}", get(obs_golden))
        // ── mTLS auto ────────────────────────────────────────
        .route("/api/mesh/automtls", get(get_auto_mtls).put(set_auto_mtls))
        .with_state(state)
}

// ─── Helpers ─────────────────────────────────────────────────

fn ok<T: serde::Serialize>(v: T) -> impl IntoResponse {
    (StatusCode::OK, Json(v))
}

fn created<T: serde::Serialize>(v: T) -> impl IntoResponse {
    (StatusCode::CREATED, Json(v))
}

fn not_found(msg: impl Into<String>) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": msg.into() })))
}

fn bad_request(msg: impl Into<String>) -> impl IntoResponse {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg.into() })))
}

// ─── Health / Metrics ────────────────────────────────────────

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "module": "cave-mesh" }))
}

async fn metrics_handler(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        s.metrics.export(),
    )
}

// ─── Service Registry ────────────────────────────────────────

async fn list_services(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.registry.list_services())
}

#[derive(Deserialize)]
struct RegisterServiceReq {
    meta: ServiceMeta,
    endpoint: crate::models::Endpoint,
}

async fn register_service(
    State(s): State<Arc<MeshState>>,
    Json(req): Json<RegisterServiceReq>,
) -> impl IntoResponse {
    s.registry.register(req.meta, req.endpoint);
    created(serde_json::json!({ "ok": true }))
}

async fn get_service(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.registry.get_service(&ns, &name) {
        Some(m) => ok(m).into_response(),
        None => not_found(format!("{ns}/{name}")).into_response(),
    }
}

async fn deregister_service(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    // Remove all endpoints by listing then deregistering — simplified
    let endpoints = s.registry.resolve_all(&format!("{ns}/{name}"));
    for e in &endpoints {
        s.registry.deregister(&ns, &name, &e.address, e.port);
    }
    ok(serde_json::json!({ "removed": endpoints.len() }))
}

#[derive(Deserialize)]
struct UpdateHealthReq {
    healthy: bool,
}

async fn update_health(
    State(s): State<Arc<MeshState>>,
    Path((ns, name, addr, port)): Path<(String, String, String, u16)>,
    Json(req): Json<UpdateHealthReq>,
) -> impl IntoResponse {
    let status = if req.healthy { HealthStatus::Healthy } else { HealthStatus::Unhealthy };
    s.registry.update_health(&ns, &name, &addr, port, status);
    ok(serde_json::json!({ "ok": true }))
}

// ─── VirtualService ──────────────────────────────────────────

async fn list_virtual_services(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.traffic.list_virtual_services())
}

async fn upsert_virtual_service(
    State(s): State<Arc<MeshState>>,
    Json(vs): Json<VirtualService>,
) -> impl IntoResponse {
    s.traffic.upsert_virtual_service(vs);
    created(serde_json::json!({ "ok": true }))
}

async fn get_virtual_service(
    State(s): State<Arc<MeshState>>,
    Path(host): Path<String>,
) -> impl IntoResponse {
    match s.traffic.get_virtual_service(&host) {
        Some(vs) => ok(vs).into_response(),
        None => not_found(host).into_response(),
    }
}

async fn delete_virtual_service(
    State(s): State<Arc<MeshState>>,
    Path(host): Path<String>,
) -> impl IntoResponse {
    s.traffic.remove_virtual_service(&host);
    ok(serde_json::json!({ "ok": true }))
}

// ─── DestinationRule ─────────────────────────────────────────

async fn list_destination_rules(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.traffic.list_destination_rules())
}

async fn upsert_destination_rule(
    State(s): State<Arc<MeshState>>,
    Json(dr): Json<DestinationRule>,
) -> impl IntoResponse {
    s.traffic.upsert_destination_rule(dr);
    created(serde_json::json!({ "ok": true }))
}

async fn get_destination_rule(
    State(s): State<Arc<MeshState>>,
    Path(host): Path<String>,
) -> impl IntoResponse {
    match s.traffic.get_destination_rule(&host) {
        Some(dr) => ok(dr).into_response(),
        None => not_found(host).into_response(),
    }
}

async fn delete_destination_rule(
    State(s): State<Arc<MeshState>>,
    Path(host): Path<String>,
) -> impl IntoResponse {
    s.traffic.remove_destination_rule(&host);
    ok(serde_json::json!({ "ok": true }))
}

// ─── Gateway ─────────────────────────────────────────────────

async fn list_gateways(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    let gws: Vec<Gateway> = s.gateways.read().unwrap().values().cloned().collect();
    ok(gws)
}

async fn upsert_gateway(
    State(s): State<Arc<MeshState>>,
    Json(gw): Json<Gateway>,
) -> impl IntoResponse {
    let key = format!("{}/{}", gw.namespace, gw.name);
    s.gateways.write().unwrap().insert(key, gw);
    created(serde_json::json!({ "ok": true }))
}

async fn get_gateway(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{ns}/{name}");
    match s.gateways.read().unwrap().get(&key).cloned() {
        Some(gw) => ok(gw).into_response(),
        None => not_found(key).into_response(),
    }
}

async fn delete_gateway(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{ns}/{name}");
    s.gateways.write().unwrap().remove(&key);
    ok(serde_json::json!({ "ok": true }))
}

// ─── ServiceEntry ─────────────────────────────────────────────

async fn list_service_entries(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    let ses: Vec<ServiceEntry> = s.service_entries.read().unwrap().values().cloned().collect();
    ok(ses)
}

async fn upsert_service_entry(
    State(s): State<Arc<MeshState>>,
    Json(se): Json<ServiceEntry>,
) -> impl IntoResponse {
    let key = format!("{}/{}", se.namespace, se.name);
    s.service_entries.write().unwrap().insert(key, se);
    created(serde_json::json!({ "ok": true }))
}

async fn get_service_entry(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{ns}/{name}");
    match s.service_entries.read().unwrap().get(&key).cloned() {
        Some(se) => ok(se).into_response(),
        None => not_found(key).into_response(),
    }
}

async fn delete_service_entry(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{ns}/{name}");
    s.service_entries.write().unwrap().remove(&key);
    ok(serde_json::json!({ "ok": true }))
}

// ─── PeerAuthentication ──────────────────────────────────────

async fn list_peer_auth(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.mtls.list_policies())
}

async fn upsert_peer_auth(
    State(s): State<Arc<MeshState>>,
    Json(pa): Json<PeerAuthentication>,
) -> impl IntoResponse {
    s.mtls.upsert_policy(pa);
    created(serde_json::json!({ "ok": true }))
}

async fn get_peer_auth(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.mtls.get_policy(&ns, &name) {
        Some(pa) => ok(pa).into_response(),
        None => not_found(format!("{ns}/{name}")).into_response(),
    }
}

async fn delete_peer_auth(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    s.mtls.remove_policy(&ns, &name);
    ok(serde_json::json!({ "ok": true }))
}

// ─── RequestAuthentication ───────────────────────────────────

async fn list_request_auth(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.auth.list_request_auth())
}

async fn upsert_request_auth(
    State(s): State<Arc<MeshState>>,
    Json(ra): Json<RequestAuthentication>,
) -> impl IntoResponse {
    s.auth.upsert_request_auth(ra);
    created(serde_json::json!({ "ok": true }))
}

async fn delete_request_auth(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    s.auth.remove_request_auth(&ns, &name);
    ok(serde_json::json!({ "ok": true }))
}

// ─── AuthorizationPolicy ─────────────────────────────────────

async fn list_authz_policies(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.auth.list_authz_policies())
}

async fn upsert_authz_policy(
    State(s): State<Arc<MeshState>>,
    Json(ap): Json<AuthorizationPolicy>,
) -> impl IntoResponse {
    s.auth.upsert_authz_policy(ap);
    created(serde_json::json!({ "ok": true }))
}

async fn delete_authz_policy(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    s.auth.remove_authz_policy(&ns, &name);
    ok(serde_json::json!({ "ok": true }))
}

// ─── Rate Limit ──────────────────────────────────────────────

async fn list_rate_limits(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.rate_limiter.list_policies())
}

async fn upsert_rate_limit(
    State(s): State<Arc<MeshState>>,
    Json(rl): Json<RateLimitPolicy>,
) -> impl IntoResponse {
    s.rate_limiter.upsert_policy(rl);
    created(serde_json::json!({ "ok": true }))
}

async fn delete_rate_limit(
    State(s): State<Arc<MeshState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    s.rate_limiter.remove_policy(&name);
    ok(serde_json::json!({ "ok": true }))
}

// ─── Circuit Breakers ────────────────────────────────────────

async fn list_circuit_breakers(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.circuit.snapshot())
}

// ─── Sidecar ─────────────────────────────────────────────────

async fn list_sidecars(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.sidecar_mgr.list())
}

async fn upsert_sidecar(
    State(s): State<Arc<MeshState>>,
    Json(sc): Json<Sidecar>,
) -> impl IntoResponse {
    s.sidecar_mgr.upsert(sc);
    created(serde_json::json!({ "ok": true }))
}

async fn get_sidecar(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.sidecar_mgr.get(&ns, &name) {
        Some(sc) => ok(sc).into_response(),
        None => not_found(format!("{ns}/{name}")).into_response(),
    }
}

async fn delete_sidecar(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    s.sidecar_mgr.remove(&ns, &name);
    ok(serde_json::json!({ "ok": true }))
}

// ─── EnvoyFilter ─────────────────────────────────────────────

async fn list_envoy_filters(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.envoy_filter_mgr.list())
}

async fn upsert_envoy_filter(
    State(s): State<Arc<MeshState>>,
    Json(ef): Json<EnvoyFilter>,
) -> impl IntoResponse {
    s.envoy_filter_mgr.upsert(ef);
    created(serde_json::json!({ "ok": true }))
}

async fn get_envoy_filter(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.envoy_filter_mgr.get(&ns, &name) {
        Some(ef) => ok(ef).into_response(),
        None => not_found(format!("{ns}/{name}")).into_response(),
    }
}

async fn delete_envoy_filter(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    s.envoy_filter_mgr.remove(&ns, &name);
    ok(serde_json::json!({ "ok": true }))
}

// ─── WorkloadGroup ───────────────────────────────────────────

async fn list_workload_groups(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.workload_group_mgr.list_groups())
}

async fn upsert_workload_group(
    State(s): State<Arc<MeshState>>,
    Json(wg): Json<WorkloadGroup>,
) -> impl IntoResponse {
    s.workload_group_mgr.upsert_group(wg);
    created(serde_json::json!({ "ok": true }))
}

async fn get_workload_group(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.workload_group_mgr.get_group(&ns, &name) {
        Some(wg) => ok(wg).into_response(),
        None => not_found(format!("{ns}/{name}")).into_response(),
    }
}

async fn delete_workload_group(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    s.workload_group_mgr.remove_group(&ns, &name);
    ok(serde_json::json!({ "ok": true }))
}

// ─── WorkloadEntry ───────────────────────────────────────────

async fn list_workload_entries(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.workload_group_mgr.list_entries())
}

async fn upsert_workload_entry(
    State(s): State<Arc<MeshState>>,
    Json(we): Json<WorkloadEntry>,
) -> impl IntoResponse {
    s.workload_group_mgr.upsert_entry(we);
    created(serde_json::json!({ "ok": true }))
}

async fn delete_workload_entry(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    s.workload_group_mgr.remove_entry(&ns, &name);
    ok(serde_json::json!({ "ok": true }))
}

// ─── Telemetry ───────────────────────────────────────────────

async fn list_telemetries(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.telemetry_mgr.list())
}

async fn upsert_telemetry(
    State(s): State<Arc<MeshState>>,
    Json(t): Json<Telemetry>,
) -> impl IntoResponse {
    s.telemetry_mgr.upsert(t);
    created(serde_json::json!({ "ok": true }))
}

async fn get_telemetry(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match s.telemetry_mgr.get(&ns, &name) {
        Some(t) => ok(t).into_response(),
        None => not_found(format!("{ns}/{name}")).into_response(),
    }
}

async fn delete_telemetry(
    State(s): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    s.telemetry_mgr.remove(&ns, &name);
    ok(serde_json::json!({ "ok": true }))
}

// ─── xDS ─────────────────────────────────────────────────────

async fn get_xds_snapshot(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.xds.default_snapshot())
}

async fn set_xds_snapshot(
    State(s): State<Arc<MeshState>>,
    Json(snapshot): Json<XdsSnapshot>,
) -> impl IntoResponse {
    s.xds.set_snapshot("_default", snapshot);
    created(serde_json::json!({ "ok": true }))
}

async fn list_xds_nodes(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.xds.list_nodes())
}

async fn xds_sync_status(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.xds.list_sync_status())
}

// ─── Multi-cluster ───────────────────────────────────────────

async fn list_clusters(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.multicluster.list_clusters())
}

async fn register_cluster(
    State(s): State<Arc<MeshState>>,
    Json(cluster): Json<RemoteCluster>,
) -> impl IntoResponse {
    s.multicluster.register_cluster(cluster);
    created(serde_json::json!({ "ok": true }))
}

async fn get_cluster(
    State(s): State<Arc<MeshState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match s.multicluster.get_cluster(&name) {
        Some(c) => ok(c).into_response(),
        None => not_found(name).into_response(),
    }
}

async fn remove_cluster(
    State(s): State<Arc<MeshState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    s.multicluster.remove_cluster(&name);
    ok(serde_json::json!({ "ok": true }))
}

async fn list_federations(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.multicluster.list_federations())
}

async fn add_federation(
    State(s): State<Arc<MeshState>>,
    Json(fed): Json<TrustDomainFederation>,
) -> impl IntoResponse {
    s.multicluster.federate(fed);
    created(serde_json::json!({ "ok": true }))
}

async fn multicluster_status(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(s.multicluster.federation_snapshot())
}

// ─── Observability ───────────────────────────────────────────

async fn obs_metrics(
    State(s): State<Arc<MeshState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.obs.request_metrics(id) {
        Some(m) => ok(m).into_response(),
        None => not_found(id.to_string()).into_response(),
    }
}

async fn obs_golden(
    State(s): State<Arc<MeshState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    ok(s.obs.golden_signals(id))
}

// ─── Auto-mTLS ───────────────────────────────────────────────

async fn get_auto_mtls(State(s): State<Arc<MeshState>>) -> impl IntoResponse {
    ok(serde_json::json!({ "auto_mtls_enabled": s.mtls.auto_mtls_enabled() }))
}

#[derive(Deserialize)]
struct SetAutoMtlsReq {
    enabled: bool,
}

async fn set_auto_mtls(
    State(s): State<Arc<MeshState>>,
    Json(req): Json<SetAutoMtlsReq>,
) -> impl IntoResponse {
    s.mtls.set_auto_mtls(req.enabled);
    ok(serde_json::json!({ "auto_mtls_enabled": req.enabled }))
}
