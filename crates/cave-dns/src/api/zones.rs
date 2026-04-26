use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use tracing::info;

use crate::{
    api::{ApiError, ApiOk, ApiResult, CreateZoneRequest, ZoneDto},
    config::{ZoneConfig, ZoneType},
    zone::ZoneManager,
};

#[derive(Clone)]
pub struct ZoneState {
    pub zones: Arc<ZoneManager>,
}

/// GET /api/v1/zones
pub async fn list_zones(State(state): State<ZoneState>) -> Json<ApiOk<Vec<ZoneDto>>> {
    let names = state.zones.zone_names();
    let mut dtos = Vec::new();
    for name in &names {
        if let Some(zone_arc) = state.zones.get_zone(name) {
            let zone = zone_arc.read().await;
            dtos.push(ZoneDto {
                name: zone.origin.to_string(),
                zone_type: format!("{:?}", zone.zone_type),
                serial: zone.serial(),
                record_count: zone.all_records().len(),
            });
        }
    }
    Json(ApiOk { data: dtos })
}

/// POST /api/v1/zones
pub async fn create_zone(
    State(state): State<ZoneState>,
    Json(req): Json<CreateZoneRequest>,
) -> Result<Json<ApiOk<ZoneDto>>, ApiError> {
    let cfg = ZoneConfig {
        name: req.name.clone(),
        file: req.file.clone(),
        zone_type: match req.zone_type.as_deref() {
            Some("secondary") => ZoneType::Secondary,
            Some("hint") => ZoneType::Hint,
            _ => ZoneType::Primary,
        },
        masters: vec![],
        tsig_key: None,
    };
    state.zones.load_zone(&cfg).await.map_err(ApiError::from)?;
    info!(zone = %req.name, "zone created via API");

    let name = req.name.parse().map_err(|e: hickory_proto::error::ProtoError| {
        ApiError {
            error: e.to_string(),
        }
    })?;
    let zone_arc = state
        .zones
        .get_zone(&name)
        .ok_or_else(|| ApiError {
            error: "zone created but not found".into(),
        })?;
    let zone = zone_arc.read().await;
    Ok(Json(ApiOk {
        data: ZoneDto {
            name: zone.origin.to_string(),
            zone_type: format!("{:?}", zone.zone_type),
            serial: zone.serial(),
            record_count: zone.all_records().len(),
        },
    }))
}

/// GET /api/v1/zones/:zone
pub async fn get_zone(
    State(state): State<ZoneState>,
    Path(zone_name): Path<String>,
) -> Result<Json<ApiOk<ZoneDto>>, ApiError> {
    let name = fqdn(&zone_name).parse().map_err(|e: hickory_proto::error::ProtoError| ApiError {
        error: e.to_string(),
    })?;
    let zone_arc = state
        .zones
        .get_zone(&name)
        .ok_or_else(|| ApiError {
            error: format!("zone {zone_name} not found"),
        })?;
    let zone = zone_arc.read().await;
    Ok(Json(ApiOk {
        data: ZoneDto {
            name: zone.origin.to_string(),
            zone_type: format!("{:?}", zone.zone_type),
            serial: zone.serial(),
            record_count: zone.all_records().len(),
        },
    }))
}

/// DELETE /api/v1/zones/:zone
pub async fn delete_zone(
    State(state): State<ZoneState>,
    Path(zone_name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let name = fqdn(&zone_name).parse().map_err(|e: hickory_proto::error::ProtoError| ApiError {
        error: e.to_string(),
    })?;
    state
        .zones
        .remove_zone(&name)
        .await
        .map_err(ApiError::from)?;
    info!(zone = %zone_name, "zone deleted via API");
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/zones/:zone/export
pub async fn export_zone(
    State(state): State<ZoneState>,
    Path(zone_name): Path<String>,
) -> Result<String, ApiError> {
    let name = fqdn(&zone_name).parse().map_err(|e: hickory_proto::error::ProtoError| ApiError {
        error: e.to_string(),
    })?;
    let zone_arc = state
        .zones
        .get_zone(&name)
        .ok_or_else(|| ApiError {
            error: format!("zone {zone_name} not found"),
        })?;
    let zone = zone_arc.read().await;

    let tmpfile = std::env::temp_dir().join(format!("{zone_name}.zone"));
    crate::zone::file::save_zone_file(&zone, &tmpfile).map_err(ApiError::from)?;
    let content = std::fs::read_to_string(&tmpfile).unwrap_or_default();
    let _ = std::fs::remove_file(&tmpfile);
    Ok(content)
}

fn fqdn(s: &str) -> String {
    if s.ends_with('.') {
        s.to_owned()
    } else {
        format!("{s}.")
    }
}
