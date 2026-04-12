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
}

#[derive(Deserialize)]
pub struct CreateRecordRequest {
    pub name: String,
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
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn list_zones(State(state): State<Arc<DnsState>>) -> impl IntoResponse {
    let zones = state.zones.list_zones();
    Json(ZoneListResponse { zones })
}

async fn create_zone(
    State(state): State<Arc<DnsState>>,
    Json(req): Json<CreateZoneRequest>,
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
}

async fn delete_zone(
    State(state): State<Arc<DnsState>>,
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
}

async fn list_records(
    State(state): State<Arc<DnsState>>,
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
}

async fn create_record(
    State(state): State<Arc<DnsState>>,
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
}

async fn delete_record(
    State(state): State<Arc<DnsState>>,
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
