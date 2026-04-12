use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use hickory_proto::rr::{DNSClass, RData, Record, RecordType};
use serde::Deserialize;
use tracing::info;
use uuid::Uuid;

use crate::{
    api::{
        ApiError, ApiOk, BatchRecordRequest, CreateRecordRequest, RecordDto, UpdateRecordRequest,
    },
    zone::ZoneManager,
};

#[derive(Clone)]
pub struct RecordState {
    pub zones: Arc<ZoneManager>,
}

#[derive(Deserialize)]
pub struct RecordFilter {
    pub name: Option<String>,
    pub r#type: Option<String>,
}

fn record_to_dto(r: &Record) -> RecordDto {
    RecordDto {
        id: Uuid::new_v4().to_string(),
        name: r.name().to_string(),
        ttl: r.ttl(),
        class: r.dns_class().to_string(),
        record_type: r.record_type().to_string(),
        rdata: r.data().map(|d| d.to_string()).unwrap_or_default(),
    }
}

fn fqdn(s: &str) -> String {
    if s.ends_with('.') {
        s.to_owned()
    } else {
        format!("{s}.")
    }
}

/// GET /api/v1/zones/:zone/records
pub async fn list_records(
    State(state): State<RecordState>,
    Path(zone_name): Path<String>,
    Query(filter): Query<RecordFilter>,
) -> Result<Json<ApiOk<Vec<RecordDto>>>, ApiError> {
    let name = fqdn(&zone_name)
        .parse()
        .map_err(|e: hickory_proto::error::ProtoError| ApiError { error: e.to_string() })?;
    let zone_arc = state
        .zones
        .get_zone(&name)
        .ok_or_else(|| ApiError {
            error: format!("zone {zone_name} not found"),
        })?;
    let zone = zone_arc.read().await;

    let mut records = zone.all_records();

    if let Some(name_filter) = &filter.name {
        let filter_n: hickory_proto::rr::Name = fqdn(name_filter)
            .parse()
            .map_err(|e: hickory_proto::error::ProtoError| ApiError { error: e.to_string() })?;
        records.retain(|r| r.name() == &filter_n);
    }
    if let Some(type_filter) = &filter.r#type {
        let rtype: RecordType = type_filter
            .parse()
            .map_err(|_| ApiError {
                error: format!("unknown type: {type_filter}"),
            })?;
        records.retain(|r| r.record_type() == rtype);
    }

    Ok(Json(ApiOk {
        data: records.iter().map(record_to_dto).collect(),
    }))
}

/// POST /api/v1/zones/:zone/records
pub async fn create_record(
    State(state): State<RecordState>,
    Path(zone_name): Path<String>,
    Json(req): Json<CreateRecordRequest>,
) -> Result<(StatusCode, Json<ApiOk<RecordDto>>), ApiError> {
    let zone_n: hickory_proto::rr::Name = fqdn(&zone_name)
        .parse()
        .map_err(|e: hickory_proto::error::ProtoError| ApiError { error: e.to_string() })?;
    let zone_arc = state
        .zones
        .get_zone(&zone_n)
        .ok_or_else(|| ApiError {
            error: format!("zone {zone_name} not found"),
        })?;

    let rtype: RecordType = req
        .record_type
        .parse()
        .map_err(|_| ApiError {
            error: format!("unknown type: {}", req.record_type),
        })?;
    let rname: hickory_proto::rr::Name = fqdn(&req.name)
        .parse()
        .map_err(|e: hickory_proto::error::ProtoError| ApiError { error: e.to_string() })?;

    let rdata = parse_rdata(rtype, &req.rdata)?;

    let mut r = Record::new();
    r.set_name(rname);
    r.set_ttl(req.ttl);
    r.set_record_type(rtype);
    r.set_dns_class(DNSClass::IN);
    r.set_data(Some(rdata));

    let dto = record_to_dto(&r);

    let mut zone = zone_arc.write().await;
    zone.add_record(r);
    info!(zone = %zone_name, name = %req.name, rtype = %req.record_type, "record created");

    Ok((StatusCode::CREATED, Json(ApiOk { data: dto })))
}

/// DELETE /api/v1/zones/:zone/records
pub async fn delete_record(
    State(state): State<RecordState>,
    Path((zone_name, record_name, rtype_str)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let zone_n: hickory_proto::rr::Name = fqdn(&zone_name)
        .parse()
        .map_err(|e: hickory_proto::error::ProtoError| ApiError { error: e.to_string() })?;
    let zone_arc = state
        .zones
        .get_zone(&zone_n)
        .ok_or_else(|| ApiError {
            error: format!("zone {zone_name} not found"),
        })?;

    let rtype: RecordType = rtype_str
        .parse()
        .map_err(|_| ApiError {
            error: format!("unknown type: {rtype_str}"),
        })?;
    let rname: hickory_proto::rr::Name = fqdn(&record_name)
        .parse()
        .map_err(|e: hickory_proto::error::ProtoError| ApiError { error: e.to_string() })?;

    let mut zone = zone_arc.write().await;
    zone.remove_record(&rname, rtype, None);
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/zones/:zone/records/batch
pub async fn batch_records(
    State(state): State<RecordState>,
    Path(zone_name): Path<String>,
    Json(req): Json<BatchRecordRequest>,
) -> Result<StatusCode, ApiError> {
    let zone_n: hickory_proto::rr::Name = fqdn(&zone_name)
        .parse()
        .map_err(|e: hickory_proto::error::ProtoError| ApiError { error: e.to_string() })?;
    let zone_arc = state
        .zones
        .get_zone(&zone_n)
        .ok_or_else(|| ApiError {
            error: format!("zone {zone_name} not found"),
        })?;

    let mut zone = zone_arc.write().await;

    for cr in req.create {
        let rtype: RecordType = cr.record_type.parse().map_err(|_| ApiError {
            error: format!("unknown type: {}", cr.record_type),
        })?;
        let rname: hickory_proto::rr::Name = fqdn(&cr.name)
            .parse()
            .map_err(|e: hickory_proto::error::ProtoError| ApiError { error: e.to_string() })?;
        let rdata = parse_rdata(rtype, &cr.rdata)?;
        let mut r = Record::new();
        r.set_name(rname);
        r.set_ttl(cr.ttl);
        r.set_record_type(rtype);
        r.set_dns_class(DNSClass::IN);
        r.set_data(Some(rdata));
        zone.add_record(r);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Best-effort rdata parser for common record types.
fn parse_rdata(rtype: RecordType, value: &str) -> Result<RData, ApiError> {
    let err = |s: &str| ApiError {
        error: format!("invalid {rtype} rdata '{value}': {s}"),
    };

    match rtype {
        RecordType::A => {
            let addr: std::net::Ipv4Addr = value.parse().map_err(|e: std::net::AddrParseError| err(&e.to_string()))?;
            Ok(RData::A(hickory_proto::rr::rdata::A(addr)))
        }
        RecordType::AAAA => {
            let addr: std::net::Ipv6Addr = value.parse().map_err(|e: std::net::AddrParseError| err(&e.to_string()))?;
            Ok(RData::AAAA(hickory_proto::rr::rdata::AAAA(addr)))
        }
        RecordType::CNAME | RecordType::NS | RecordType::PTR => {
            let name: hickory_proto::rr::Name = fqdn(value)
                .parse()
                .map_err(|e: hickory_proto::error::ProtoError| err(&e.to_string()))?;
            Ok(match rtype {
                RecordType::CNAME => RData::CNAME(hickory_proto::rr::rdata::CNAME(name)),
                RecordType::NS => RData::NS(hickory_proto::rr::rdata::NS(name)),
                RecordType::PTR => RData::PTR(hickory_proto::rr::rdata::PTR(name)),
                _ => unreachable!(),
            })
        }
        RecordType::TXT => Ok(RData::TXT(hickory_proto::rr::rdata::TXT::new(vec![
            value.to_string(),
        ]))),
        RecordType::MX => {
            let mut parts = value.splitn(2, ' ');
            let pref: u16 = parts
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| err("expected 'priority exchange'"))?;
            let exchange: hickory_proto::rr::Name = parts
                .next()
                .ok_or_else(|| err("missing exchange"))?
                .parse()
                .map_err(|e: hickory_proto::error::ProtoError| err(&e.to_string()))?;
            Ok(RData::MX(hickory_proto::rr::rdata::MX::new(pref, exchange)))
        }
        _ => Err(ApiError {
            error: format!("unsupported record type for API: {rtype}"),
        }),
    }
}
