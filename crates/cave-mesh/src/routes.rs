//! HTTP routes for cave-mesh — CRUD for services, policies, virtual services,
//! destination rules, service entries; topology and metrics endpoints.

use crate::{
    models::*,
    mtls, observability,
    proxy::{self, CircuitBreakerState},
    traffic, MeshState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

pub fn create_router(state: Arc<MeshState>) -> Router {
    Router::new()
        // ── Services ──────────────────────────────────────────────────────
        .route("/api/v1/mesh/services", get(list_services).post(create_service))
        .route(
            "/api/v1/mesh/services/{id}",
            get(get_service).put(update_service).delete(delete_service),
        )
        .route(
            "/api/v1/mesh/services/{id}/metrics",
            get(get_service_metrics),
        )
        .route(
            "/api/v1/mesh/services/{id}/instances",
            get(list_instances).post(register_instance),
        )
        .route(
            "/api/v1/mesh/services/{id}/instances/{iid}",
            delete(deregister_instance),
        )
        // ── Virtual Services ──────────────────────────────────────────────
        .route(
            "/api/v1/mesh/virtual-services",
            get(list_virtual_services).post(create_virtual_service),
        )
        .route(
            "/api/v1/mesh/virtual-services/{id}",
            get(get_virtual_service)
                .put(update_virtual_service)
                .delete(delete_virtual_service),
        )
        .route(
            "/api/v1/mesh/virtual-services/{id}/traffic-split",
            get(get_traffic_split),
        )
        // ── Traffic Policies ──────────────────────────────────────────────
        .route(
            "/api/v1/mesh/traffic-policies",
            get(list_traffic_policies).post(create_traffic_policy),
        )
        .route(
            "/api/v1/mesh/traffic-policies/{id}",
            get(get_traffic_policy)
                .put(update_traffic_policy)
                .delete(delete_traffic_policy),
        )
        // ── Destination Rules ─────────────────────────────────────────────
        .route(
            "/api/v1/mesh/destination-rules",
            get(list_destination_rules).post(create_destination_rule),
        )
        .route(
            "/api/v1/mesh/destination-rules/{id}",
            get(get_destination_rule).delete(delete_destination_rule),
        )
        // ── Service Entries ───────────────────────────────────────────────
        .route(
            "/api/v1/mesh/service-entries",
            get(list_service_entries).post(create_service_entry),
        )
        .route(
            "/api/v1/mesh/service-entries/{id}",
            get(get_service_entry).delete(delete_service_entry),
        )
        // ── Topology ──────────────────────────────────────────────────────
        .route("/api/v1/mesh/topology", get(get_topology))
        // ── Circuit Breakers ──────────────────────────────────────────────
        .route("/api/v1/mesh/circuit-breakers", get(list_circuit_breakers))
        .route(
            "/api/v1/mesh/circuit-breakers/{service_id}/probe",
            post(probe_circuit_breaker),
        )
        // ── mTLS Certs ────────────────────────────────────────────────────
        .route("/api/v1/mesh/certs", get(list_certs))
        .route(
            "/api/v1/mesh/certs/{service_id}/generate",
            post(generate_cert),
        )
        .route(
            "/api/v1/mesh/certs/{service_id}/rotate",
            post(rotate_cert),
        )
        .route(
            "/api/v1/mesh/certs/{cert_id}/verify",
            get(verify_cert),
        )
        // ── Proxy / Routing ───────────────────────────────────────────────
        .route("/api/v1/mesh/route", post(resolve_route))
        // ── Fault Injection ───────────────────────────────────────────────
        .route(
            "/api/v1/mesh/virtual-services/{id}/fault-inject",
            post(evaluate_fault_injection),
        )
        // ── Health ────────────────────────────────────────────────────────
        .route("/api/v1/mesh/health", get(health))
        .with_state(state)
}

// ─── Error helper ─────────────────────────────────────────────────────────────

fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
}

fn bad_request(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
}

// ─── Services ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateServiceRequest {
    pub name: String,
    pub namespace: String,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub ports: Vec<ServicePort>,
    pub protocol: Protocol,
}

async fn list_services(State(state): State<Arc<MeshState>>) -> Json<Vec<Service>> {
    let services = state.services.lock().unwrap();
    Json(services.values().cloned().collect())
}

async fn create_service(
    State(state): State<Arc<MeshState>>,
    Json(req): Json<CreateServiceRequest>,
) -> Json<Service> {
    let now = Utc::now();
    let svc = Service {
        id: Uuid::new_v4(),
        name: req.name,
        namespace: req.namespace,
        labels: req.labels,
        ports: req.ports,
        protocol: req.protocol,
        created_at: now,
        updated_at: now,
    };
    state.services.lock().unwrap().insert(svc.id, svc.clone());
    tracing::info!(service_id = %svc.id, name = %svc.name, "Registered service");
    Json(svc)
}

async fn get_service(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<Service>, (StatusCode, Json<serde_json::Value>)> {
    let services = state.services.lock().unwrap();
    services
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found(&format!("Service {id} not found")))
}

async fn update_service(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
    Json(req): Json<CreateServiceRequest>,
) -> Result<Json<Service>, (StatusCode, Json<serde_json::Value>)> {
    let mut services = state.services.lock().unwrap();
    if let Some(svc) = services.get_mut(&id) {
        svc.name = req.name;
        svc.namespace = req.namespace;
        svc.labels = req.labels;
        svc.ports = req.ports;
        svc.protocol = req.protocol;
        svc.updated_at = Utc::now();
        Ok(Json(svc.clone()))
    } else {
        Err(not_found(&format!("Service {id} not found")))
    }
}

async fn delete_service(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let removed = state.services.lock().unwrap().remove(&id).is_some();
    if removed {
        Ok(Json(serde_json::json!({ "deleted": id })))
    } else {
        Err(not_found(&format!("Service {id} not found")))
    }
}

// ─── Service Metrics ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ServiceMetricsResponse {
    pub service_id: Uuid,
    pub service_name: String,
    pub request_metrics: Option<observability::RequestMetricsResponse>,
    pub latency_histogram: Option<observability::LatencyBuckets>,
    pub error_rate: f64,
    pub golden_signals: observability::GoldenSignals,
}

async fn get_service_metrics(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<ServiceMetricsResponse>, (StatusCode, Json<serde_json::Value>)> {
    let name = {
        let services = state.services.lock().unwrap();
        services
            .get(&id)
            .map(|s| s.name.clone())
            .ok_or_else(|| not_found(&format!("Service {id} not found")))?
    };
    Ok(Json(ServiceMetricsResponse {
        service_id: id,
        service_name: name,
        request_metrics: observability::request_metrics(id, &state),
        latency_histogram: observability::latency_histogram(id, &state),
        error_rate: observability::error_rate(id, &state),
        golden_signals: observability::golden_signals(id, &state),
    }))
}

// ─── Instances ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterInstanceRequest {
    pub address: String,
    pub port: u16,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub version: Option<String>,
}

fn default_weight() -> u32 { 100 }

async fn list_instances(
    Path(service_id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<ServiceInstance>> {
    let instances = state.instances.lock().unwrap();
    Json(
        instances
            .values()
            .filter(|i| i.service_id == service_id)
            .cloned()
            .collect(),
    )
}

async fn register_instance(
    Path(service_id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
    Json(req): Json<RegisterInstanceRequest>,
) -> Result<Json<ServiceInstance>, (StatusCode, Json<serde_json::Value>)> {
    {
        let services = state.services.lock().unwrap();
        if !services.contains_key(&service_id) {
            return Err(not_found(&format!("Service {service_id} not found")));
        }
    }
    let instance = ServiceInstance {
        id: Uuid::new_v4(),
        service_id,
        address: req.address,
        port: req.port,
        weight: req.weight,
        health: HealthStatus::Healthy,
        labels: req.labels,
        version: req.version,
        registered_at: Utc::now(),
    };
    state
        .instances
        .lock()
        .unwrap()
        .insert(instance.id, instance.clone());
    Ok(Json(instance))
}

async fn deregister_instance(
    Path((service_id, instance_id)): Path<(Uuid, Uuid)>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut instances = state.instances.lock().unwrap();
    let exists = instances
        .get(&instance_id)
        .map(|i| i.service_id == service_id)
        .unwrap_or(false);
    if exists {
        instances.remove(&instance_id);
        Ok(Json(serde_json::json!({ "deleted": instance_id })))
    } else {
        Err(not_found(&format!("Instance {instance_id} not found")))
    }
}

// ─── Virtual Services ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateVirtualServiceRequest {
    pub name: String,
    pub hosts: Vec<String>,
    #[serde(default)]
    pub http_routes: Vec<HttpRoute>,
    #[serde(default)]
    pub tls_routes: Vec<TlsRoute>,
    pub fault_injection: Option<FaultInjection>,
}

async fn list_virtual_services(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<VirtualService>> {
    let vs = state.virtual_services.lock().unwrap();
    Json(vs.values().cloned().collect())
}

async fn create_virtual_service(
    State(state): State<Arc<MeshState>>,
    Json(req): Json<CreateVirtualServiceRequest>,
) -> Json<VirtualService> {
    let now = Utc::now();
    let vs = VirtualService {
        id: Uuid::new_v4(),
        name: req.name,
        hosts: req.hosts,
        http_routes: req.http_routes,
        tls_routes: req.tls_routes,
        fault_injection: req.fault_injection,
        created_at: now,
        updated_at: now,
    };
    state
        .virtual_services
        .lock()
        .unwrap()
        .insert(vs.id, vs.clone());
    Json(vs)
}

async fn get_virtual_service(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<VirtualService>, (StatusCode, Json<serde_json::Value>)> {
    let vs = state.virtual_services.lock().unwrap();
    vs.get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found(&format!("VirtualService {id} not found")))
}

async fn update_virtual_service(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
    Json(req): Json<CreateVirtualServiceRequest>,
) -> Result<Json<VirtualService>, (StatusCode, Json<serde_json::Value>)> {
    let mut vs_map = state.virtual_services.lock().unwrap();
    if let Some(vs) = vs_map.get_mut(&id) {
        vs.name = req.name;
        vs.hosts = req.hosts;
        vs.http_routes = req.http_routes;
        vs.tls_routes = req.tls_routes;
        vs.fault_injection = req.fault_injection;
        vs.updated_at = Utc::now();
        Ok(Json(vs.clone()))
    } else {
        Err(not_found(&format!("VirtualService {id} not found")))
    }
}

async fn delete_virtual_service(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if state.virtual_services.lock().unwrap().remove(&id).is_some() {
        Ok(Json(serde_json::json!({ "deleted": id })))
    } else {
        Err(not_found(&format!("VirtualService {id} not found")))
    }
}

async fn get_traffic_split(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<Vec<traffic::TrafficSplitResult>>, (StatusCode, Json<serde_json::Value>)> {
    let vs_map = state.virtual_services.lock().unwrap();
    let vs = vs_map
        .get(&id)
        .ok_or_else(|| not_found(&format!("VirtualService {id} not found")))?;
    Ok(Json(traffic::traffic_split(vs)))
}

// ─── Traffic Policies ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateTrafficPolicyRequest {
    pub name: String,
    pub service_id: Uuid,
    pub retry_policy: Option<RetryPolicy>,
    pub timeout: Option<TimeoutPolicy>,
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    pub rate_limit: Option<RateLimitPolicy>,
}

async fn list_traffic_policies(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<TrafficPolicy>> {
    let policies = state.traffic_policies.lock().unwrap();
    Json(policies.values().cloned().collect())
}

async fn create_traffic_policy(
    State(state): State<Arc<MeshState>>,
    Json(req): Json<CreateTrafficPolicyRequest>,
) -> Json<TrafficPolicy> {
    let now = Utc::now();
    let policy = TrafficPolicy {
        id: Uuid::new_v4(),
        name: req.name,
        service_id: req.service_id,
        retry_policy: req.retry_policy,
        timeout: req.timeout,
        circuit_breaker: req.circuit_breaker,
        rate_limit: req.rate_limit,
        created_at: now,
        updated_at: now,
    };
    state
        .traffic_policies
        .lock()
        .unwrap()
        .insert(policy.id, policy.clone());
    Json(policy)
}

async fn get_traffic_policy(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<TrafficPolicy>, (StatusCode, Json<serde_json::Value>)> {
    let policies = state.traffic_policies.lock().unwrap();
    policies
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found(&format!("TrafficPolicy {id} not found")))
}

async fn update_traffic_policy(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
    Json(req): Json<CreateTrafficPolicyRequest>,
) -> Result<Json<TrafficPolicy>, (StatusCode, Json<serde_json::Value>)> {
    let mut policies = state.traffic_policies.lock().unwrap();
    if let Some(p) = policies.get_mut(&id) {
        p.name = req.name;
        p.service_id = req.service_id;
        p.retry_policy = req.retry_policy;
        p.timeout = req.timeout;
        p.circuit_breaker = req.circuit_breaker;
        p.rate_limit = req.rate_limit;
        p.updated_at = Utc::now();
        Ok(Json(p.clone()))
    } else {
        Err(not_found(&format!("TrafficPolicy {id} not found")))
    }
}

async fn delete_traffic_policy(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if state.traffic_policies.lock().unwrap().remove(&id).is_some() {
        Ok(Json(serde_json::json!({ "deleted": id })))
    } else {
        Err(not_found(&format!("TrafficPolicy {id} not found")))
    }
}

// ─── Destination Rules ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateDestinationRuleRequest {
    pub name: String,
    pub host: String,
    pub traffic_policy: Option<TrafficPolicySpec>,
    #[serde(default)]
    pub subsets: Vec<Subset>,
    pub mtls: Option<MtlsConfig>,
}

async fn list_destination_rules(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<DestinationRule>> {
    let dr = state.destination_rules.lock().unwrap();
    Json(dr.values().cloned().collect())
}

async fn create_destination_rule(
    State(state): State<Arc<MeshState>>,
    Json(req): Json<CreateDestinationRuleRequest>,
) -> Json<DestinationRule> {
    let now = Utc::now();
    let dr = DestinationRule {
        id: Uuid::new_v4(),
        name: req.name,
        host: req.host,
        traffic_policy: req.traffic_policy,
        subsets: req.subsets,
        mtls: req.mtls,
        created_at: now,
        updated_at: now,
    };
    state
        .destination_rules
        .lock()
        .unwrap()
        .insert(dr.id, dr.clone());
    Json(dr)
}

async fn get_destination_rule(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<DestinationRule>, (StatusCode, Json<serde_json::Value>)> {
    let dr = state.destination_rules.lock().unwrap();
    dr.get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found(&format!("DestinationRule {id} not found")))
}

async fn delete_destination_rule(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if state.destination_rules.lock().unwrap().remove(&id).is_some() {
        Ok(Json(serde_json::json!({ "deleted": id })))
    } else {
        Err(not_found(&format!("DestinationRule {id} not found")))
    }
}

// ─── Service Entries ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateServiceEntryRequest {
    pub name: String,
    pub hosts: Vec<String>,
    #[serde(default)]
    pub addresses: Vec<String>,
    #[serde(default)]
    pub ports: Vec<ServicePort>,
    pub location: ServiceLocation,
    pub resolution: ServiceResolution,
    #[serde(default)]
    pub endpoints: Vec<ServiceEndpoint>,
}

async fn list_service_entries(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<ServiceEntry>> {
    let entries = state.service_entries.lock().unwrap();
    Json(entries.values().cloned().collect())
}

async fn create_service_entry(
    State(state): State<Arc<MeshState>>,
    Json(req): Json<CreateServiceEntryRequest>,
) -> Json<ServiceEntry> {
    let now = Utc::now();
    let entry = ServiceEntry {
        id: Uuid::new_v4(),
        name: req.name,
        hosts: req.hosts,
        addresses: req.addresses,
        ports: req.ports,
        location: req.location,
        resolution: req.resolution,
        endpoints: req.endpoints,
        created_at: now,
        updated_at: now,
    };
    state
        .service_entries
        .lock()
        .unwrap()
        .insert(entry.id, entry.clone());
    Json(entry)
}

async fn get_service_entry(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<ServiceEntry>, (StatusCode, Json<serde_json::Value>)> {
    let entries = state.service_entries.lock().unwrap();
    entries
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found(&format!("ServiceEntry {id} not found")))
}

async fn delete_service_entry(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if state.service_entries.lock().unwrap().remove(&id).is_some() {
        Ok(Json(serde_json::json!({ "deleted": id })))
    } else {
        Err(not_found(&format!("ServiceEntry {id} not found")))
    }
}

// ─── Topology ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct TopologyResponse {
    pub services: Vec<Service>,
    pub instances: Vec<ServiceInstance>,
    pub virtual_services: Vec<VirtualService>,
    pub destination_rules: Vec<DestinationRule>,
    pub service_entries: Vec<ServiceEntry>,
    pub circuit_breakers: Vec<CircuitBreakerState>,
    pub service_count: usize,
    pub instance_count: usize,
}

async fn get_topology(State(state): State<Arc<MeshState>>) -> Json<TopologyResponse> {
    let services: Vec<Service> = state.services.lock().unwrap().values().cloned().collect();
    let instances: Vec<ServiceInstance> =
        state.instances.lock().unwrap().values().cloned().collect();
    let virtual_services: Vec<VirtualService> =
        state.virtual_services.lock().unwrap().values().cloned().collect();
    let destination_rules: Vec<DestinationRule> =
        state.destination_rules.lock().unwrap().values().cloned().collect();
    let service_entries: Vec<ServiceEntry> =
        state.service_entries.lock().unwrap().values().cloned().collect();
    let circuit_breakers: Vec<CircuitBreakerState> =
        state.circuit_breakers.lock().unwrap().values().cloned().collect();

    Json(TopologyResponse {
        service_count: services.len(),
        instance_count: instances.len(),
        services,
        instances,
        virtual_services,
        destination_rules,
        service_entries,
        circuit_breakers,
    })
}

// ─── Circuit Breakers ─────────────────────────────────────────────────────────

async fn list_circuit_breakers(
    State(state): State<Arc<MeshState>>,
) -> Json<Vec<CircuitBreakerState>> {
    let breakers = state.circuit_breakers.lock().unwrap();
    Json(breakers.values().cloned().collect())
}

#[derive(Deserialize)]
pub struct ProbeRequest {
    pub success: bool,
    #[serde(default = "default_threshold")]
    pub threshold: u32,
}
fn default_threshold() -> u32 { 5 }

async fn probe_circuit_breaker(
    Path(service_id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
    Json(req): Json<ProbeRequest>,
) -> Json<CircuitBreakerState> {
    proxy::record_outcome(service_id, req.success, req.threshold, &state);
    let breakers = state.circuit_breakers.lock().unwrap();
    let cb = breakers
        .get(&service_id)
        .cloned()
        .unwrap_or_else(|| CircuitBreakerState::new(service_id));
    Json(cb)
}

// ─── mTLS Certificates ────────────────────────────────────────────────────────

async fn list_certs(State(state): State<Arc<MeshState>>) -> Json<Vec<mtls::CertInventoryEntry>> {
    Json(mtls::cert_inventory(&state))
}

#[derive(Deserialize)]
pub struct GenerateCertRequest {
    pub namespace: Option<String>,
    #[serde(default = "default_validity_days")]
    pub validity_days: i64,
}
fn default_validity_days() -> i64 { 90 }

async fn generate_cert(
    Path(service_id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
    Json(req): Json<GenerateCertRequest>,
) -> Result<Json<mtls::CertRecord>, (StatusCode, Json<serde_json::Value>)> {
    let (service_name, namespace) = {
        let services = state.services.lock().unwrap();
        let svc = services
            .get(&service_id)
            .ok_or_else(|| not_found(&format!("Service {service_id} not found")))?;
        (
            svc.name.clone(),
            req.namespace.unwrap_or_else(|| svc.namespace.clone()),
        )
    };
    mtls::generate_cert(service_id, &service_name, &namespace, req.validity_days, &state)
        .map(Json)
        .map_err(|e| bad_request(&e))
}

async fn rotate_cert(
    Path(service_id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<mtls::CertRecord>, (StatusCode, Json<serde_json::Value>)> {
    let (service_name, namespace) = {
        let services = state.services.lock().unwrap();
        let svc = services
            .get(&service_id)
            .ok_or_else(|| not_found(&format!("Service {service_id} not found")))?;
        (svc.name.clone(), svc.namespace.clone())
    };
    mtls::rotate_cert(service_id, &service_name, &namespace, &state)
        .map(Json)
        .map_err(|e| bad_request(&e))
}

async fn verify_cert(
    Path(cert_id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    mtls::verify_peer(cert_id, &state)
        .map(|valid| Json(serde_json::json!({ "cert_id": cert_id, "valid": valid })))
        .map_err(|e| not_found(&e))
}

// ─── Proxy / Routing ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RouteRequest {
    pub host: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

async fn resolve_route(
    State(state): State<Arc<MeshState>>,
    Json(req): Json<RouteRequest>,
) -> Result<Json<proxy::RouteDecision>, (StatusCode, Json<serde_json::Value>)> {
    proxy::route_request(&req.host, &req.method, &req.path, &req.headers, &state)
        .map(Json)
        .ok_or_else(|| not_found("No matching route or healthy instance found"))
}

// ─── Fault Injection Evaluation ───────────────────────────────────────────────

async fn evaluate_fault_injection(
    Path(id): Path<Uuid>,
    State(state): State<Arc<MeshState>>,
) -> Result<Json<traffic::FaultInjectionResult>, (StatusCode, Json<serde_json::Value>)> {
    let vs_map = state.virtual_services.lock().unwrap();
    let vs = vs_map
        .get(&id)
        .ok_or_else(|| not_found(&format!("VirtualService {id} not found")))?;
    match &vs.fault_injection {
        None => Ok(Json(traffic::fault_injection(&FaultInjection {
            delay: None,
            abort: None,
        }))),
        Some(fi) => Ok(Json(traffic::fault_injection(fi))),
    }
}

// ─── Health ───────────────────────────────────────────────────────────────────

async fn health(State(state): State<Arc<MeshState>>) -> Json<serde_json::Value> {
    let service_count = state.services.lock().unwrap().len();
    let instance_count = state.instances.lock().unwrap().len();
    let vs_count = state.virtual_services.lock().unwrap().len();
    let cert_count = state.certs.lock().unwrap().len();
    Json(serde_json::json!({
        "module": "cave-mesh",
        "status": "ok",
        "upstream": "Istio + Linkerd",
        "stats": {
            "services": service_count,
            "instances": instance_count,
            "virtual_services": vs_count,
            "certs": cert_count,
        }
    }))
}
