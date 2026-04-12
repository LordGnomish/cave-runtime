//! Admin API: CRUD for all mesh resources.
//!
//! Endpoint map:
//!   Services            GET/POST /api/mesh/services
//!                       GET/DELETE /api/mesh/services/:ns/:name
//!                       PUT /api/mesh/services/:ns/:name/health/:addr/:port
//!   VirtualServices     GET/POST /api/mesh/virtualservices
//!                       GET/DELETE /api/mesh/virtualservices/:host
//!   DestinationRules    GET/POST /api/mesh/destinationrules
//!                       GET/DELETE /api/mesh/destinationrules/:host
//!   Gateways            GET/POST /api/mesh/gateways
//!                       GET/DELETE /api/mesh/gateways/:ns/:name
//!   ServiceEntries      GET/POST /api/mesh/serviceentries
//!                       GET/DELETE /api/mesh/serviceentries/:ns/:name
//!   PeerAuth            GET/POST /api/mesh/peerauthentications
//!                       DELETE /api/mesh/peerauthentications/:ns/:name
//!   RequestAuth         GET/POST /api/mesh/requestauthentications
//!                       DELETE /api/mesh/requestauthentications/:ns/:name
//!   AuthzPolicy         GET/POST /api/mesh/authorizationpolicies
//!                       DELETE /api/mesh/authorizationpolicies/:ns/:name
//!   RateLimit           GET/POST /api/mesh/ratelimits
//!                       DELETE /api/mesh/ratelimits/:name
//!   CircuitBreakers     GET /api/mesh/circuitbreakers
//!   Metrics             GET /api/mesh/metrics
//!   Health              GET /api/mesh/health

use crate::{
    models::{
        AuthorizationPolicy, DestinationRule, Endpoint, Gateway, HealthStatus,
        PeerAuthentication, RateLimitPolicy, RequestAuthentication, ServiceEntry, ServiceMeta,
        VirtualService,
    },
    MeshState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, put},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────
// Router
// ─────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<MeshState>) -> Router {
    Router::new()
        // ── Health / Metrics ─────────────────────────────────
        .route("/api/mesh/health", get(health))
        .route("/api/mesh/metrics", get(metrics_handler))
        // ── Service Registry ─────────────────────────────────
        .route("/api/mesh/services", get(list_services).post(register_service))
        .route(
            "/api/mesh/services/:ns/:name",
            get(get_service).delete(deregister_service),
        )
        .route(
            "/api/mesh/services/:ns/:name/health/:addr/:port",
            put(update_health),
        )
        // ── VirtualServices ──────────────────────────────────
        .route(
            "/api/mesh/virtualservices",
            get(list_virtual_services).post(upsert_virtual_service),
        )
        .route(
            "/api/mesh/virtualservices/:host",
            get(get_virtual_service).delete(delete_virtual_service),
        )
        // ── DestinationRules ─────────────────────────────────
        .route(
            "/api/mesh/destinationrules",
            get(list_destination_rules).post(upsert_destination_rule),
        )
        .route(
            "/api/mesh/destinationrules/:host",
            get(get_destination_rule).delete(delete_destination_rule),
        )
        // ── Gateways ─────────────────────────────────────────
        .route(
            "/api/mesh/gateways",
            get(list_gateways).post(upsert_gateway),
        )
        .route(
            "/api/mesh/gateways/:ns/:name",
            get(get_gateway).delete(delete_gateway),
        )
        // ── ServiceEntries ───────────────────────────────────
        .route(
            "/api/mesh/serviceentries",
            get(list_service_entries).post(upsert_service_entry),
        )
        .route(
            "/api/mesh/serviceentries/:ns/:name",
            get(get_service_entry).delete(delete_service_entry),
        )
        // ── PeerAuthentication ───────────────────────────────
        .route(
            "/api/mesh/peerauthentications",
            get(list_peer_authentications).post(upsert_peer_authentication),
        )
        .route(
            "/api/mesh/peerauthentications/:ns/:name",
            delete(delete_peer_authentication),
        )
        // ── RequestAuthentication ────────────────────────────
        .route(
            "/api/mesh/requestauthentications",
            get(list_request_authentications).post(upsert_request_authentication),
        )
        .route(
            "/api/mesh/requestauthentications/:ns/:name",
            delete(delete_request_authentication),
        )
        // ── AuthorizationPolicy ──────────────────────────────
        .route(
            "/api/mesh/authorizationpolicies",
            get(list_authz_policies).post(upsert_authz_policy),
        )
        .route(
            "/api/mesh/authorizationpolicies/:ns/:name",
            delete(delete_authz_policy),
        )
        // ── Rate Limiting ────────────────────────────────────
        .route(
            "/api/mesh/ratelimits",
            get(list_rate_limits).post(upsert_rate_limit),
        )
        .route("/api/mesh/ratelimits/:name", delete(delete_rate_limit))
        // ── Circuit Breakers ─────────────────────────────────
        .route("/api/mesh/circuitbreakers", get(list_circuit_breakers))
        .with_state(state)
}

// ─────────────────────────────────────────────────────────────
// Health / Metrics
// ─────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-mesh",
        "status": "ok",
        "upstream": "Istio",
        "features": [
            "service-discovery", "virtual-services", "destination-rules",
            "gateways", "service-entries", "mtls", "jwt-validation",
            "authz-policy", "rate-limiting", "circuit-breaking",
            "fault-injection", "retries", "tracing"
        ]
    }))
}

async fn metrics_handler(State(state): State<Arc<MeshState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        state.metrics.export(),
    )
}

// ─────────────────────────────────────────────────────────────
// Service Registry
// ─────────────────────────────────────────────────────────────

async fn list_services(State(state): State<Arc<MeshState>>) -> Json<Vec<ServiceMeta>> {
    Json(state.registry.list_services())
}

#[derive(Debug, Deserialize)]
struct RegisterServiceRequest {
    meta: ServiceMeta,
    endpoint: Endpoint,
}

async fn register_service(
    State(state): State<Arc<MeshState>>,
    Json(req): Json<RegisterServiceRequest>,
) -> StatusCode {
    state.registry.register(req.meta, req.endpoint);
    StatusCode::CREATED
}

async fn get_service(
    State(state): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.registry.get_service(&ns, &name) {
        Some(meta) => Json(meta).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn deregister_service(
    State(state): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    // Remove all endpoints (deregister entire service)
    let endpoints = state.registry.resolve_all(&format!("{ns}/{name}"));
    for ep in endpoints {
        state
            .registry
            .deregister(&ns, &name, &ep.address, ep.port);
    }
    StatusCode::NO_CONTENT
}

#[derive(Debug, Deserialize)]
struct UpdateHealthRequest {
    status: String,
}

async fn update_health(
    State(state): State<Arc<MeshState>>,
    Path((ns, name, addr, port)): Path<(String, String, String, u16)>,
    Json(req): Json<UpdateHealthRequest>,
) -> StatusCode {
    let status = match req.status.as_str() {
        "healthy" => HealthStatus::Healthy,
        "unhealthy" => HealthStatus::Unhealthy,
        _ => HealthStatus::Unknown,
    };
    state.registry.update_health(&ns, &name, &addr, port, status);
    StatusCode::NO_CONTENT
}

// ─────────────────────────────────────────────────────────────
// VirtualServices
// ─────────────────────────────────────────────────────────────

async fn list_virtual_services(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<VirtualService>> {
    Json(state.traffic.list_virtual_services())
}

async fn upsert_virtual_service(
    State(state): State<Arc<MeshState>>,
    Json(mut vs): Json<VirtualService>,
) -> StatusCode {
    vs.updated_at = Utc::now();
    state.traffic.upsert_virtual_service(vs);
    StatusCode::CREATED
}

async fn get_virtual_service(
    State(state): State<Arc<MeshState>>,
    Path(host): Path<String>,
) -> impl IntoResponse {
    match state.traffic.get_virtual_service(&host) {
        Some(vs) => Json(vs).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_virtual_service(
    State(state): State<Arc<MeshState>>,
    Path(host): Path<String>,
) -> StatusCode {
    state.traffic.remove_virtual_service(&host);
    StatusCode::NO_CONTENT
}

// ─────────────────────────────────────────────────────────────
// DestinationRules
// ─────────────────────────────────────────────────────────────

async fn list_destination_rules(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<DestinationRule>> {
    Json(state.traffic.list_destination_rules())
}

async fn upsert_destination_rule(
    State(state): State<Arc<MeshState>>,
    Json(mut dr): Json<DestinationRule>,
) -> StatusCode {
    dr.updated_at = Utc::now();
    state.traffic.upsert_destination_rule(dr);
    StatusCode::CREATED
}

async fn get_destination_rule(
    State(state): State<Arc<MeshState>>,
    Path(host): Path<String>,
) -> impl IntoResponse {
    match state.traffic.get_destination_rule(&host) {
        Some(dr) => Json(dr).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_destination_rule(
    State(state): State<Arc<MeshState>>,
    Path(host): Path<String>,
) -> StatusCode {
    state.traffic.remove_destination_rule(&host);
    StatusCode::NO_CONTENT
}

// ─────────────────────────────────────────────────────────────
// Gateways
// ─────────────────────────────────────────────────────────────

// Gateways are stored in MeshState directly.
async fn list_gateways(State(state): State<Arc<MeshState>>) -> Json<Vec<Gateway>> {
    let map = state.gateways.read().unwrap();
    Json(map.values().cloned().collect())
}

async fn upsert_gateway(
    State(state): State<Arc<MeshState>>,
    Json(mut gw): Json<Gateway>,
) -> StatusCode {
    gw.updated_at = Utc::now();
    let key = format!("{}/{}", gw.namespace, gw.name);
    state.gateways.write().unwrap().insert(key, gw);
    StatusCode::CREATED
}

async fn get_gateway(
    State(state): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{ns}/{name}");
    let map = state.gateways.read().unwrap();
    match map.get(&key) {
        Some(gw) => Json(gw.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_gateway(
    State(state): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    let key = format!("{ns}/{name}");
    state.gateways.write().unwrap().remove(&key);
    StatusCode::NO_CONTENT
}

// ─────────────────────────────────────────────────────────────
// ServiceEntries
// ─────────────────────────────────────────────────────────────

async fn list_service_entries(State(state): State<Arc<MeshState>>) -> Json<Vec<ServiceEntry>> {
    let map = state.service_entries.read().unwrap();
    Json(map.values().cloned().collect())
}

async fn upsert_service_entry(
    State(state): State<Arc<MeshState>>,
    Json(mut se): Json<ServiceEntry>,
) -> StatusCode {
    se.updated_at = Utc::now();
    let key = format!("{}/{}", se.namespace, se.name);
    state.service_entries.write().unwrap().insert(key, se);
    StatusCode::CREATED
}

async fn get_service_entry(
    State(state): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{ns}/{name}");
    let map = state.service_entries.read().unwrap();
    match map.get(&key) {
        Some(se) => Json(se.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_service_entry(
    State(state): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    let key = format!("{ns}/{name}");
    state.service_entries.write().unwrap().remove(&key);
    StatusCode::NO_CONTENT
}

// ─────────────────────────────────────────────────────────────
// PeerAuthentication
// ─────────────────────────────────────────────────────────────

async fn list_peer_authentications(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<PeerAuthentication>> {
    Json(state.mtls.list_policies())
}

async fn upsert_peer_authentication(
    State(state): State<Arc<MeshState>>,
    Json(mut pa): Json<PeerAuthentication>,
) -> StatusCode {
    pa.updated_at = Utc::now();
    state.mtls.upsert_policy(pa);
    StatusCode::CREATED
}

async fn delete_peer_authentication(
    State(state): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    state.mtls.remove_policy(&ns, &name);
    StatusCode::NO_CONTENT
}

// ─────────────────────────────────────────────────────────────
// RequestAuthentication
// ─────────────────────────────────────────────────────────────

async fn list_request_authentications(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<RequestAuthentication>> {
    Json(state.auth.list_request_auth())
}

async fn upsert_request_authentication(
    State(state): State<Arc<MeshState>>,
    Json(mut ra): Json<RequestAuthentication>,
) -> StatusCode {
    ra.updated_at = Utc::now();
    state.auth.upsert_request_auth(ra);
    StatusCode::CREATED
}

async fn delete_request_authentication(
    State(state): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    state.auth.remove_request_auth(&ns, &name);
    StatusCode::NO_CONTENT
}

// ─────────────────────────────────────────────────────────────
// AuthorizationPolicy
// ─────────────────────────────────────────────────────────────

async fn list_authz_policies(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<AuthorizationPolicy>> {
    Json(state.auth.list_authz_policies())
}

async fn upsert_authz_policy(
    State(state): State<Arc<MeshState>>,
    Json(mut ap): Json<AuthorizationPolicy>,
) -> StatusCode {
    ap.updated_at = Utc::now();
    state.auth.upsert_authz_policy(ap);
    StatusCode::CREATED
}

async fn delete_authz_policy(
    State(state): State<Arc<MeshState>>,
    Path((ns, name)): Path<(String, String)>,
) -> StatusCode {
    state.auth.remove_authz_policy(&ns, &name);
    StatusCode::NO_CONTENT
}

// ─────────────────────────────────────────────────────────────
// Rate Limiting
// ─────────────────────────────────────────────────────────────

async fn list_rate_limits(State(state): State<Arc<MeshState>>) -> Json<Vec<RateLimitPolicy>> {
    Json(state.rate_limiter.list_policies())
}

async fn upsert_rate_limit(
    State(state): State<Arc<MeshState>>,
    Json(mut rl): Json<RateLimitPolicy>,
) -> StatusCode {
    rl.updated_at = Utc::now();
    state.rate_limiter.upsert_policy(rl);
    StatusCode::CREATED
}

async fn delete_rate_limit(
    State(state): State<Arc<MeshState>>,
    Path(name): Path<String>,
) -> StatusCode {
    state.rate_limiter.remove_policy(&name);
    StatusCode::NO_CONTENT
}

// ─────────────────────────────────────────────────────────────
// Circuit Breakers
// ─────────────────────────────────────────────────────────────

async fn list_circuit_breakers(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<crate::circuit::BreakerSnapshot>> {
    Json(state.circuit.snapshot())
}
