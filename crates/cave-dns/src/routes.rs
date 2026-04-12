<<<<<<< HEAD
//! HTTP routes for cave-dns.

use crate::{
    manager,
    models::{DnsProvider, DnsRecord, DnsZone},
    DnsState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub fn create_router(state: Arc<DnsState>) -> Router {
    Router::new()
        // Zones
        .route("/api/v1/dns/zones", get(list_zones).post(create_zone))
        .route(
            "/api/v1/dns/zones/{id}",
            get(get_zone).put(update_zone).delete(delete_zone),
        )
        // Records
        .route(
            "/api/v1/dns/zones/{id}/records",
            get(list_records).post(create_record),
        )
        .route(
            "/api/v1/dns/zones/{id}/records/{record_id}",
            get(get_record).put(update_record).delete(delete_record),
        )
        // Drift detection
        .route("/api/v1/dns/zones/{id}/drift", get(get_drift))
        // Sync
        .route("/api/v1/dns/zones/{id}/sync", post(sync_zone))
        // Providers
        .route("/api/v1/dns/providers", get(list_providers))
        // Health
        .route("/api/v1/dns/health", get(health))
        .with_state(state)
}

// ── Request / Response DTOs ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateZoneRequest {
    pub name: String,
    pub provider: DnsProvider,
    pub ttl_default: u32,
=======
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;

use crate::cache::DnsCache;
use crate::discovery::{ServiceEndpoint, ServiceRegistry};
use crate::resolver::Resolver;
use crate::types::*;
use crate::zone::{Zone, ZoneStore};

// ── State ────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub struct DnsState {
    pub zones: Arc<ZoneStore>,
    pub registry: Arc<ServiceRegistry>,
    pub resolver: Arc<Resolver>,
}

// ── Request/Response types ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateZoneRequest {
    pub origin: String,
    pub ttl: Option<u32>,
>>>>>>> claude/dazzling-tesla
}

#[derive(Deserialize)]
pub struct CreateRecordRequest {
    pub name: String,
<<<<<<< HEAD
    pub record_type: crate::models::RecordType,
    pub ttl: u32,
    pub data: crate::models::RecordData,
}

#[derive(Deserialize)]
pub struct SyncRequest {
    pub dry_run: bool,
}

#[derive(Serialize)]
pub struct SyncResponse {
    pub zone_id: Uuid,
    pub changes_applied: usize,
    pub dry_run: bool,
    pub errors: Vec<String>,
}

#[derive(Serialize)]
pub struct ProviderInfo {
    pub name: &'static str,
    pub slug: &'static str,
=======
    #[serde(rename = "type")]
    pub rtype: String,
    pub ttl: Option<u32>,
    pub address: Option<String>,
    pub target: Option<String>,
    pub priority: Option<u16>,
    pub port: Option<u16>,
    pub weight: Option<u16>,
    pub text: Option<String>,
}

#[derive(Serialize)]
pub struct ZoneListResponse {
    pub zones: Vec<String>,
}

#[derive(Serialize)]
pub struct RecordListResponse {
    pub records: Vec<ResourceRecord>,
}

#[derive(Deserialize)]
pub struct LookupQuery {
    pub name: String,
    #[serde(rename = "type")]
    pub rtype: Option<String>,
}

#[derive(Deserialize)]
pub struct NamespaceParams {
    pub namespace: String,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub module: &'static str,
}

// ── Router ───────────────────────────────────────────────────────────────────

pub fn router(state: Arc<DnsState>) -> Router {
    Router::new()
        .route("/api/dns/zones", get(list_zones).post(create_zone))
        .route("/api/dns/zones/:zone", delete(delete_zone))
        .route(
            "/api/dns/zones/:zone/records",
            get(list_records).post(create_record),
        )
        .route(
            "/api/dns/zones/:zone/records/:name/:rtype",
            delete(delete_record),
        )
        .route("/api/dns/lookup", get(lookup))
        .route("/api/dns/discovery/register", post(register_service))
        .route("/api/dns/discovery/:fqdn", delete(deregister_service))
        .route("/api/dns/discovery/ns/:namespace", get(list_services))
        .route("/api/dns/health", get(health))
        .with_state(state)
>>>>>>> claude/dazzling-tesla
}

// ── Handlers ─────────────────────────────────────────────────────────────────

<<<<<<< HEAD
async fn list_zones(State(state): State<Arc<DnsState>>) -> Json<Vec<DnsZone>> {
    Json(state.zones.lock().unwrap().clone())
=======
async fn list_zones(State(state): State<Arc<DnsState>>) -> impl IntoResponse {
    let zones = state.zones.list_zones();
    Json(ZoneListResponse { zones })
>>>>>>> claude/dazzling-tesla
}

async fn create_zone(
    State(state): State<Arc<DnsState>>,
    Json(req): Json<CreateZoneRequest>,
<<<<<<< HEAD
) -> (StatusCode, Json<DnsZone>) {
    let zone = DnsZone::new(req.name, req.provider, req.ttl_default);
    state.zones.lock().unwrap().push(zone.clone());
    (StatusCode::CREATED, Json(zone))
}

async fn get_zone(
    State(state): State<Arc<DnsState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<DnsZone>, StatusCode> {
    state
        .zones
        .lock()
        .unwrap()
        .iter()
        .find(|z| z.id == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_zone(
    State(state): State<Arc<DnsState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateZoneRequest>,
) -> Result<Json<DnsZone>, StatusCode> {
    let mut zones = state.zones.lock().unwrap();
    let zone = zones.iter_mut().find(|z| z.id == id).ok_or(StatusCode::NOT_FOUND)?;
    zone.name = req.name;
    zone.provider = req.provider;
    zone.ttl_default = req.ttl_default;
    zone.updated_at = Utc::now();
    Ok(Json(zone.clone()))
=======
) -> impl IntoResponse {
    let ttl = req.ttl.unwrap_or(3600);
    let origin = if req.origin.ends_with('.') {
        req.origin.clone()
    } else {
        format!("{}.", req.origin)
    };

    let soa = ResourceRecord {
        name: origin.clone(),
        rtype: RecordType::SOA,
        class: CLASS_IN,
        ttl,
        rdata: RData::SOA {
            mname: format!("ns1.{}", origin),
            rname: format!("admin.{}", origin),
            serial: 1,
            refresh: 3600,
            retry: 900,
            expire: 604800,
            minimum: 300,
        },
    };

    let zone = Zone {
        origin: origin.clone(),
        soa,
        records: HashMap::new(),
    };

    match state.zones.add_zone(zone) {
        Ok(_) => (StatusCode::CREATED, Json(serde_json::json!({"origin": origin}))).into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
>>>>>>> claude/dazzling-tesla
}

async fn delete_zone(
    State(state): State<Arc<DnsState>>,
<<<<<<< HEAD
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut zones = state.zones.lock().unwrap();
    let before = zones.len();
    zones.retain(|z| z.id != id);
    if zones.len() < before { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
=======
    Path(zone): Path<String>,
) -> impl IntoResponse {
    match state.zones.remove_zone(&zone) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
>>>>>>> claude/dazzling-tesla
}

async fn list_records(
    State(state): State<Arc<DnsState>>,
<<<<<<< HEAD
    Path(id): Path<Uuid>,
) -> Json<Vec<DnsRecord>> {
    let records = state.records.lock().unwrap();
    Json(records.iter().filter(|r| r.zone_id == id).cloned().collect())
=======
    Path(zone): Path<String>,
) -> impl IntoResponse {
    match state.zones.get_zone(&zone) {
        Some(z) => {
            let records: Vec<ResourceRecord> =
                z.records.into_values().flatten().collect();
            Json(RecordListResponse { records }).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "zone not found"})),
        )
            .into_response(),
    }
>>>>>>> claude/dazzling-tesla
}

async fn create_record(
    State(state): State<Arc<DnsState>>,
<<<<<<< HEAD
    Path(id): Path<Uuid>,
    Json(req): Json<CreateRecordRequest>,
) -> (StatusCode, Json<DnsRecord>) {
    let record = DnsRecord::new(id, req.name, req.record_type, req.ttl, req.data);
    state.records.lock().unwrap().push(record.clone());
    (StatusCode::CREATED, Json(record))
}

async fn get_record(
    State(state): State<Arc<DnsState>>,
    Path((zone_id, record_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<DnsRecord>, StatusCode> {
    state
        .records
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.zone_id == zone_id && r.id == record_id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_record(
    State(state): State<Arc<DnsState>>,
    Path((zone_id, record_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<CreateRecordRequest>,
) -> Result<Json<DnsRecord>, StatusCode> {
    let mut records = state.records.lock().unwrap();
    let record = records
        .iter_mut()
        .find(|r| r.zone_id == zone_id && r.id == record_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    record.name = req.name;
    record.record_type = req.record_type;
    record.ttl = req.ttl;
    record.data = req.data;
    record.updated_at = Utc::now();
    Ok(Json(record.clone()))
=======
    Path(zone): Path<String>,
    Json(req): Json<CreateRecordRequest>,
) -> impl IntoResponse {
    let ttl = req.ttl.unwrap_or(3600);
    let rtype = RecordType::from_str(&req.rtype);

    let zone_obj = match state.zones.get_zone(&zone) {
        Some(z) => z,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "zone not found"})),
            )
                .into_response();
        }
    };

    let name = if req.name.ends_with('.') {
        req.name.clone()
    } else {
        format!("{}.{}", req.name, zone_obj.origin)
    };

    let rdata = match &rtype {
        RecordType::A => {
            let addr = req.address.as_deref().unwrap_or("0.0.0.0");
            match addr.parse::<Ipv4Addr>() {
                Ok(ip) => RData::A(ip),
                Err(_) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "invalid IPv4 address"})),
                    )
                        .into_response();
                }
            }
        }
        RecordType::CNAME => RData::CNAME(req.target.clone().unwrap_or_default()),
        RecordType::MX => RData::MX {
            priority: req.priority.unwrap_or(10),
            exchange: req.target.clone().unwrap_or_default(),
        },
        RecordType::TXT => RData::TXT(vec![req.text.clone().unwrap_or_default().into_bytes()]),
        _ => RData::Raw(vec![]),
    };

    let record = ResourceRecord {
        name: name.clone(),
        rtype,
        class: CLASS_IN,
        ttl,
        rdata,
    };

    match state.zones.add_record(&zone, record) {
        Ok(_) => (StatusCode::CREATED, Json(serde_json::json!({"name": name}))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
>>>>>>> claude/dazzling-tesla
}

async fn delete_record(
    State(state): State<Arc<DnsState>>,
<<<<<<< HEAD
    Path((zone_id, record_id)): Path<(Uuid, Uuid)>,
) -> StatusCode {
    let mut records = state.records.lock().unwrap();
    let before = records.len();
    records.retain(|r| !(r.zone_id == zone_id && r.id == record_id));
    if records.len() < before { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
}

async fn get_drift(
    State(state): State<Arc<DnsState>>,
    Path(id): Path<Uuid>,
) -> Json<Vec<crate::models::DnsDrift>> {
    let records = state.records.lock().unwrap();
    let zone_records: Vec<DnsRecord> =
        records.iter().filter(|r| r.zone_id == id).cloned().collect();
    // Managed records are the desired state; all records are the actual state.
    let managed: Vec<DnsRecord> = zone_records.iter().filter(|r| r.managed).cloned().collect();
    Json(manager::detect_drift(&managed, &zone_records))
}

async fn sync_zone(
    State(state): State<Arc<DnsState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<SyncRequest>,
) -> Json<SyncResponse> {
    let zone_records: Vec<DnsRecord> = {
        let records = state.records.lock().unwrap();
        records.iter().filter(|r| r.zone_id == id).cloned().collect()
    };

    let scratch = Arc::new(Mutex::new(Vec::new()));
    let result = manager::sync_records(id, &zone_records, &zone_records, &scratch, req.dry_run);

    Json(SyncResponse {
        zone_id: id,
        changes_applied: result.changes_applied,
        dry_run: req.dry_run,
        errors: result.errors,
    })
}

async fn list_providers() -> Json<Vec<ProviderInfo>> {
    Json(vec![
        ProviderInfo { name: "Cloudflare", slug: "cloudflare" },
        ProviderInfo { name: "Amazon Route 53", slug: "route53" },
        ProviderInfo { name: "Azure DNS", slug: "azure" },
    ])
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-dns",
        "status": "ok",
        "upstream": "external-dns",
    }))
}
=======
    Path((zone, name, rtype)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let rtype = RecordType::from_str(&rtype);
    match state.zones.remove_record(&zone, &name, &rtype) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn lookup(
    State(state): State<Arc<DnsState>>,
    Query(params): Query<LookupQuery>,
) -> impl IntoResponse {
    let rtype = RecordType::from_str(params.rtype.as_deref().unwrap_or("A"));
    let records = state.zones.lookup(&params.name, &rtype);
    Json(serde_json::json!({ "name": params.name, "records": records }))
}

async fn register_service(
    State(state): State<Arc<DnsState>>,
    Json(endpoint): Json<ServiceEndpointJson>,
) -> impl IntoResponse {
    let ep = ServiceEndpoint {
        name: endpoint.name,
        namespace: endpoint.namespace,
        cluster_domain: endpoint.cluster_domain,
        ip: endpoint.ip.parse().unwrap_or(Ipv4Addr::UNSPECIFIED),
        port: endpoint.port,
        protocol: endpoint.protocol,
        ttl: endpoint.ttl.unwrap_or(30),
    };
    state.registry.register(ep);
    StatusCode::CREATED
}

async fn deregister_service(
    State(state): State<Arc<DnsState>>,
    Path(fqdn): Path<String>,
) -> impl IntoResponse {
    state.registry.deregister(&fqdn);
    StatusCode::NO_CONTENT
}

async fn list_services(
    State(state): State<Arc<DnsState>>,
    Path(namespace): Path<String>,
) -> impl IntoResponse {
    let services = state.registry.lookup_all_in_namespace(&namespace);
    let items: Vec<ServiceEndpointJson> = services
        .into_iter()
        .map(|ep| ServiceEndpointJson {
            name: ep.name,
            namespace: ep.namespace,
            cluster_domain: ep.cluster_domain,
            ip: ep.ip.to_string(),
            port: ep.port,
            protocol: ep.protocol,
            ttl: Some(ep.ttl),
        })
        .collect();
    Json(serde_json::json!({ "services": items }))
}

async fn health() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        module: crate::MODULE_NAME,
    })
}

// JSON-friendly service endpoint (ip as string)
#[derive(Serialize, Deserialize)]
pub struct ServiceEndpointJson {
    pub name: String,
    pub namespace: String,
    pub cluster_domain: String,
    pub ip: String,
    pub port: u16,
    pub protocol: String,
    pub ttl: Option<u32>,
}

// Make DnsCache accessible (though not used in routes directly here)
#[allow(dead_code)]
fn _use_cache(_: &DnsCache) {}
>>>>>>> claude/dazzling-tesla
