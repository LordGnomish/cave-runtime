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
}

#[derive(Deserialize)]
pub struct CreateRecordRequest {
    pub name: String,
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
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn list_zones(State(state): State<Arc<DnsState>>) -> Json<Vec<DnsZone>> {
    Json(state.zones.lock().unwrap().clone())
}

async fn create_zone(
    State(state): State<Arc<DnsState>>,
    Json(req): Json<CreateZoneRequest>,
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
}

async fn delete_zone(
    State(state): State<Arc<DnsState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut zones = state.zones.lock().unwrap();
    let before = zones.len();
    zones.retain(|z| z.id != id);
    if zones.len() < before { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
}

async fn list_records(
    State(state): State<Arc<DnsState>>,
    Path(id): Path<Uuid>,
) -> Json<Vec<DnsRecord>> {
    let records = state.records.lock().unwrap();
    Json(records.iter().filter(|r| r.zone_id == id).cloned().collect())
}

async fn create_record(
    State(state): State<Arc<DnsState>>,
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
}

async fn delete_record(
    State(state): State<Arc<DnsState>>,
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
