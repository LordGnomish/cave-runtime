// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::{
    models::{ClusterTopology, InstanceRole, InstanceStatus, RaftState, RuntimeInstance},
    HaState,
};
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct JoinRequest {
    pub hostname: String,
    pub ip: String,
    pub port: u16,
    pub datacenter: String,
}

#[derive(Deserialize)]
pub struct LeaveRequest {
    pub instance_id: Uuid,
}

#[derive(Deserialize)]
pub struct FailoverRequest {
    pub target_id: Uuid,
    pub reason: Option<String>,
}

#[derive(Deserialize)]
pub struct HandoffRequest {
    pub target_id: Uuid,
}

#[derive(Deserialize)]
pub struct DRFailoverRequest {
    pub target_site: String,
}

#[derive(Deserialize)]
pub struct DRTestRequest {
    pub simulate_site: Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<HaState>) -> Router {
    Router::new()
        .route("/api/v1/ha/status",       get(cluster_status))
        .route("/api/v1/ha/topology",     get(topology))
        .route("/api/v1/ha/join",         post(join))
        .route("/api/v1/ha/leave",        post(leave))
        .route("/api/v1/ha/failover",     post(manual_failover))
        .route("/api/v1/ha/handoff",      post(leader_handoff))
        .route("/api/v1/ha/replication",  get(replication_status))
        .route("/api/v1/ha/dr/status",    get(dr_status))
        .route("/api/v1/ha/dr/failover",  post(dr_failover))
        .route("/api/v1/ha/dr/test",      post(dr_test))
        .route("/api/v1/ha/health",       get(ha_health))
        .route("/api/v1/ha/raft/state",   get(raft_state))
        .with_state(state)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /api/v1/ha/status — cluster summary: leader, term, instance count.
async fn cluster_status(State(state): State<Arc<HaState>>) -> Json<serde_json::Value> {
    let raft = state.raft.read().await;
    let topology = state.topology.read().await;
    Json(serde_json::json!({
        "self_id":       state.self_instance.id.to_string(),
        "leader":        topology.leader.map(|id| id.to_string()),
        "current_term":  raft.current_term,
        "instances":     topology.instances.len(),
        "quorum_size":   topology.quorum_size,
        "split_brain_protection": topology.split_brain_protection,
    }))
}

/// GET /api/v1/ha/topology — full cluster topology with all instances.
async fn topology(State(state): State<Arc<HaState>>) -> Json<ClusterTopology> {
    Json(state.topology.read().await.clone())
}

/// POST /api/v1/ha/join — new instance joins the cluster.
async fn join(
    State(state): State<Arc<HaState>>,
    Json(req): Json<JoinRequest>,
) -> Json<serde_json::Value> {
    let new_instance = RuntimeInstance {
        id: Uuid::new_v4(),
        hostname: req.hostname,
        ip: req.ip,
        port: req.port,
        role: InstanceRole::Follower,
        status: InstanceStatus::Healthy,
        last_heartbeat: Utc::now(),
        datacenter: req.datacenter,
        started_at: Utc::now(),
    };
    let id = new_instance.id;
    {
        let mut topology = state.topology.write().await;
        topology.instances.push(new_instance);
        topology.quorum_size = (topology.instances.len() / 2) + 1;
    }
    Json(serde_json::json!({
        "joined":      true,
        "instance_id": id.to_string(),
    }))
}

/// POST /api/v1/ha/leave — graceful instance departure.
async fn leave(
    State(state): State<Arc<HaState>>,
    Json(req): Json<LeaveRequest>,
) -> Json<serde_json::Value> {
    let mut topology = state.topology.write().await;
    let before = topology.instances.len();
    topology.instances.retain(|i| i.id != req.instance_id);
    let removed = topology.instances.len() < before;
    if !topology.instances.is_empty() {
        topology.quorum_size = (topology.instances.len() / 2) + 1;
    }
    Json(serde_json::json!({
        "removed":     removed,
        "instance_id": req.instance_id.to_string(),
        "remaining":   topology.instances.len(),
    }))
}

/// POST /api/v1/ha/failover — manual (unplanned) leader failover.
async fn manual_failover(
    State(state): State<Arc<HaState>>,
    Json(req): Json<FailoverRequest>,
) -> Json<serde_json::Value> {
    let reason = req.reason.unwrap_or_else(|| "manual_trigger".to_string());
    match crate::failover::trigger_failover(state, req.target_id, reason).await {
        Ok(()) => Json(serde_json::json!({
            "failover": "triggered",
            "new_leader": req.target_id.to_string(),
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// POST /api/v1/ha/handoff — planned leader transfer (maintenance window).
async fn leader_handoff(
    State(state): State<Arc<HaState>>,
    Json(req): Json<HandoffRequest>,
) -> Json<serde_json::Value> {
    match crate::failover::graceful_handoff(state, req.target_id).await {
        Ok(()) => Json(serde_json::json!({
            "handoff":    "completed",
            "new_leader": req.target_id.to_string(),
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// GET /api/v1/ha/replication — replication mode and per-follower lag.
async fn replication_status(State(state): State<Arc<HaState>>) -> Json<serde_json::Value> {
    let raft = state.raft.read().await;
    let topology = state.topology.read().await;
    let followers: Vec<_> = topology
        .instances
        .iter()
        .filter(|i| matches!(i.role, InstanceRole::Follower))
        .map(|i| serde_json::json!({
            "id":       i.id.to_string(),
            "hostname": i.hostname,
            "status":   format!("{:?}", i.status),
        }))
        .collect();
    Json(serde_json::json!({
        "mode":            format!("{:?}", state.replication_config.mode),
        "commit_index":    raft.commit_index,
        "last_applied":    raft.last_applied,
        "lag_tolerance_ms": state.replication_config.lag_tolerance,
        "followers":       followers,
    }))
}

/// GET /api/v1/ha/dr/status — DR pair configuration and site health.
async fn dr_status(State(state): State<Arc<HaState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "dr_configured": state.dr_config.is_some(),
        "config": state.dr_config,
    }))
}

/// POST /api/v1/ha/dr/failover — promote secondary datacenter to primary.
async fn dr_failover(
    State(state): State<Arc<HaState>>,
    Json(req): Json<DRFailoverRequest>,
) -> Json<serde_json::Value> {
    match crate::dr::site_failover(state, req.target_site.clone()).await {
        Ok(()) => Json(serde_json::json!({
            "site_failover": "initiated",
            "target_site":   req.target_site,
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// POST /api/v1/ha/dr/test — non-destructive DR drill; rolls back after verification.
async fn dr_test(
    State(state): State<Arc<HaState>>,
    Json(req): Json<DRTestRequest>,
) -> Json<serde_json::Value> {
    let site = req.simulate_site.unwrap_or_else(|| "secondary".to_string());
    match crate::dr::dr_test(state, site).await {
        Ok(result) => Json(result),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// GET /api/v1/ha/health — aggregated cluster health summary.
async fn ha_health(State(state): State<Arc<HaState>>) -> Json<serde_json::Value> {
    match crate::health::cluster_health(state).await {
        Ok(health) => Json(health),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// GET /api/v1/ha/raft/state — Raft internal state for debugging.
async fn raft_state(State(state): State<Arc<HaState>>) -> Json<RaftState> {
    Json(state.raft.read().await.clone())
}
