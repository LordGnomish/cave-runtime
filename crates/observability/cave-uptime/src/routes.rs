// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP route handlers for the cave-uptime API.
//!
//! Implements Uptime Kuma-compatible REST endpoints:
//!   GET    /api/uptime/health
//!   GET    /api/uptime/probes
//!   POST   /api/uptime/probes
//!   GET    /api/uptime/probes/:id
//!   PUT    /api/uptime/probes/:id
//!   DELETE /api/uptime/probes/:id
//!   GET    /api/uptime/probes/:id/history
//!   GET    /api/uptime/probes/:id/stats
//!   GET    /api/uptime/status
//!   POST   /api/uptime/push/:id    (push heartbeat)

use crate::history::UptimeWindow;
use crate::models::{ProbeType, UptimeProbe};
use crate::probe::build_probe_result;
use crate::status::{MonitorStatus, ProbeStatusSummary, build_status_page};
use crate::AppState;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ─── Request / response types ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateProbeRequest {
    pub name: String,
    pub target_url: String,
    pub probe_type: ProbeType,
    pub interval_seconds: u32,
    pub timeout_ms: u32,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProbeRequest {
    pub name: Option<String>,
    pub target_url: Option<String>,
    pub interval_seconds: Option<u32>,
    pub timeout_ms: Option<u32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ProbeResponse {
    pub id: String,
    pub name: String,
    pub target_url: String,
    pub probe_type: String,
    pub interval_seconds: u32,
    pub timeout_ms: u32,
    pub enabled: bool,
}

impl From<UptimeProbe> for ProbeResponse {
    fn from(p: UptimeProbe) -> Self {
        ProbeResponse {
            id: p.id.to_string(),
            name: p.name,
            target_url: p.target_url,
            probe_type: format!("{:?}", p.probe_type).to_lowercase(),
            interval_seconds: p.interval_seconds,
            timeout_ms: p.timeout_ms,
            enabled: p.enabled,
        }
    }
}

// ─── Router factory ───────────────────────────────────────────────────────────

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/uptime/health", get(health))
        .route("/api/uptime/probes", get(list_probes).post(create_probe))
        .route(
            "/api/uptime/probes/{id}",
            get(get_probe).put(update_probe).delete(delete_probe),
        )
        .route("/api/uptime/probes/{id}/history", get(get_probe_history))
        .route("/api/uptime/probes/{id}/stats", get(get_probe_stats))
        .route("/api/uptime/status", get(get_status_page))
        .route("/api/uptime/push/{id}", post(push_heartbeat))
        .with_state(state)
}

// ─── Handlers ────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-uptime",
        "status": "ok",
        "upstream": "Uptime Kuma"
    }))
}

async fn list_probes(State(state): State<Arc<AppState>>) -> Json<Vec<ProbeResponse>> {
    let probes: Vec<ProbeResponse> = state
        .probes
        .list()
        .into_iter()
        .map(ProbeResponse::from)
        .collect();
    Json(probes)
}

async fn create_probe(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProbeRequest>,
) -> (StatusCode, Json<ProbeResponse>) {
    let probe = UptimeProbe {
        id: Uuid::new_v4(),
        name: body.name,
        target_url: body.target_url,
        probe_type: body.probe_type,
        interval_seconds: body.interval_seconds,
        timeout_ms: body.timeout_ms,
        enabled: body.enabled,
    };
    state.scheduler.register(probe.clone());
    state.probes.insert(probe.clone());
    (StatusCode::CREATED, Json(ProbeResponse::from(probe)))
}

async fn get_probe(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ProbeResponse>, StatusCode> {
    state
        .probes
        .get(id)
        .map(|p| Json(ProbeResponse::from(p)))
        .ok_or(StatusCode::NOT_FOUND)
}

async fn update_probe(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateProbeRequest>,
) -> Result<Json<ProbeResponse>, StatusCode> {
    let mut probe = state.probes.get(id).ok_or(StatusCode::NOT_FOUND)?;
    if let Some(name) = body.name {
        probe.name = name;
    }
    if let Some(url) = body.target_url {
        probe.target_url = url;
    }
    if let Some(interval) = body.interval_seconds {
        probe.interval_seconds = interval;
    }
    if let Some(timeout) = body.timeout_ms {
        probe.timeout_ms = timeout;
    }
    if let Some(enabled) = body.enabled {
        probe.enabled = enabled;
    }
    state.probes.update(probe.clone());
    state.scheduler.update(probe.clone());
    Ok(Json(ProbeResponse::from(probe)))
}

async fn delete_probe(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if state.probes.delete(id) {
        state.scheduler.unregister(id);
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn get_probe_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if state.probes.get(id).is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let history = state.heartbeats.get_history(id, 100);
    Ok(Json(serde_json::json!({
        "probe_id": id,
        "count": history.len(),
        "results": history
    })))
}

async fn get_probe_stats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if state.probes.get(id).is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let stats_24h = state.heartbeats.window_stats(id, UptimeWindow::Hours24);
    let stats_7d = state.heartbeats.window_stats(id, UptimeWindow::Days7);
    let stats_30d = state.heartbeats.window_stats(id, UptimeWindow::Days30);

    Ok(Json(serde_json::json!({
        "probe_id": id,
        "windows": {
            "24h": {
                "uptime_pct": stats_24h.uptime_pct,
                "total_checks": stats_24h.total_checks,
                "avg_latency_ms": stats_24h.avg_latency_ms
            },
            "7d": {
                "uptime_pct": stats_7d.uptime_pct,
                "total_checks": stats_7d.total_checks,
                "avg_latency_ms": stats_7d.avg_latency_ms
            },
            "30d": {
                "uptime_pct": stats_30d.uptime_pct,
                "total_checks": stats_30d.total_checks,
                "avg_latency_ms": stats_30d.avg_latency_ms
            }
        }
    })))
}

async fn get_status_page(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let probes = state.probes.list();
    let summaries: Vec<ProbeStatusSummary> = probes
        .into_iter()
        .map(|p| {
            let stats = state.heartbeats.window_stats(p.id, UptimeWindow::Hours24);
            let history = state.heartbeats.get_history(p.id, 1);
            let last_status = history
                .last()
                .map(|r| {
                    if r.success {
                        MonitorStatus::Up
                    } else {
                        MonitorStatus::Down
                    }
                })
                .unwrap_or(MonitorStatus::Pending);

            ProbeStatusSummary {
                probe_id: p.id,
                name: p.name,
                status: last_status,
                uptime_24h: stats.uptime_pct,
                avg_latency_ms: stats.avg_latency_ms,
                last_check_ms: history.last().map(|r| r.latency_ms).unwrap_or(0),
            }
        })
        .collect();

    let page = build_status_page("Cave Uptime", summaries);
    Json(serde_json::json!({
        "title": page.title,
        "all_operational": page.all_operational(),
        "up_count": page.up_count(),
        "down_count": page.down_count(),
        "overall_uptime_24h": page.overall_uptime_24h(),
        "entries": page.entries.iter().map(|e| serde_json::json!({
            "probe_id": e.summary.probe_id,
            "name": e.summary.name,
            "status": e.summary.status.label(),
            "uptime_24h": e.summary.uptime_24h,
        })).collect::<Vec<_>>()
    }))
}

/// POST /api/uptime/push/:id
///
/// Receive a passive push heartbeat from the monitored service.
async fn push_heartbeat(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if state.probes.get(id).is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    // Record a synthetic success result for the push
    let result = build_probe_result(id, true, 0, None, None);
    state.heartbeats.record(result);
    Ok(Json(serde_json::json!({
        "ok": true,
        "probe_id": id,
        "received_at": Utc::now().to_rfc3339()
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_response_from_probe() {
        let p = UptimeProbe {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            target_url: "https://x.com".to_string(),
            probe_type: ProbeType::Http,
            interval_seconds: 60,
            timeout_ms: 5000,
            enabled: true,
        };
        let r = ProbeResponse::from(p.clone());
        assert_eq!(r.name, "test");
        assert_eq!(r.probe_type, "http");
    }
}
