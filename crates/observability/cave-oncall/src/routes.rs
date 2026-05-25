// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-oncall.

use crate::models::*;
use axum::{
    Json, Router,
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------

pub struct OnCallStore {
    pub teams: Arc<RwLock<HashMap<Uuid, Team>>>,
    pub users: Arc<RwLock<HashMap<Uuid, User>>>,
    pub schedules: Arc<RwLock<HashMap<Uuid, Schedule>>>,
    pub rotations: Arc<RwLock<HashMap<Uuid, Rotation>>>,
    pub overrides: Arc<RwLock<HashMap<Uuid, ShiftOverride>>>,
    pub policies: Arc<RwLock<HashMap<Uuid, EscalationPolicy>>>,
    pub alerts: Arc<RwLock<HashMap<Uuid, Alert>>>,
    pub silences: Arc<RwLock<HashMap<Uuid, Silence>>>,
    pub incident_links: Arc<RwLock<HashMap<Uuid, IncidentLink>>>,
}

impl OnCallStore {
    pub fn new() -> Self {
        Self {
            teams: Arc::new(RwLock::new(HashMap::new())),
            users: Arc::new(RwLock::new(HashMap::new())),
            schedules: Arc::new(RwLock::new(HashMap::new())),
            rotations: Arc::new(RwLock::new(HashMap::new())),
            overrides: Arc::new(RwLock::new(HashMap::new())),
            policies: Arc::new(RwLock::new(HashMap::new())),
            alerts: Arc::new(RwLock::new(HashMap::new())),
            silences: Arc::new(RwLock::new(HashMap::new())),
            incident_links: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for OnCallStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Query param helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct AlertQuery {
    state: Option<String>,
    team: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CalendarQuery {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpcomingQuery {
    days: Option<u32>,
}

// ---------------------------------------------------------------------------
// Error response helper
// ---------------------------------------------------------------------------

fn error_response(status: StatusCode, message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        status,
        Json(serde_json::json!({
            "error": message
        })),
    )
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn create_router(state: Arc<OnCallStore>) -> Router {
    Router::new()
        // Health
        .route("/api/oncall/health", get(health))
        // Teams
        .route("/api/oncall/teams", get(list_teams))
        .route("/api/oncall/teams", post(create_team))
        .route("/api/oncall/teams/{id}", get(get_team))
        .route("/api/oncall/teams/{id}", put(update_team))
        .route("/api/oncall/teams/{id}", delete(delete_team))
        .route("/api/oncall/teams/{id}/current", get(team_current_oncall))
        .route("/api/oncall/teams/{id}/upcoming", get(team_upcoming_shifts))
        // Users
        .route("/api/oncall/users", get(list_users))
        .route("/api/oncall/users", post(create_user))
        .route("/api/oncall/users/{id}", get(get_user))
        // Schedules
        .route("/api/oncall/schedules", get(list_schedules))
        .route("/api/oncall/schedules", post(create_schedule))
        .route("/api/oncall/schedules/{id}", get(get_schedule))
        .route(
            "/api/oncall/schedules/{id}/calendar",
            get(schedule_calendar),
        )
        .route("/api/oncall/schedules/{id}", delete(delete_schedule))
        // Rotations
        .route(
            "/api/oncall/schedules/{id}/rotations",
            post(create_rotation),
        )
        .route("/api/oncall/schedules/{id}/rotations", get(list_rotations))
        .route("/api/oncall/rotations/{id}", delete(delete_rotation))
        // Overrides
        .route(
            "/api/oncall/schedules/{id}/overrides",
            post(create_override),
        )
        .route("/api/oncall/schedules/{id}/overrides", get(list_overrides))
        .route("/api/oncall/overrides/{id}", delete(delete_override))
        // Escalation Policies
        .route("/api/oncall/policies", get(list_policies))
        .route("/api/oncall/policies", post(create_policy))
        .route("/api/oncall/policies/{id}", get(get_policy))
        .route("/api/oncall/policies/{id}", delete(delete_policy))
        // Alerts
        .route("/api/oncall/alerts", get(list_alerts))
        .route("/api/oncall/alerts", post(create_alert))
        .route("/api/oncall/alerts/{id}", get(get_alert))
        .route("/api/oncall/alerts/{id}/ack", post(ack_alert))
        .route("/api/oncall/alerts/{id}/resolve", post(resolve_alert))
        .route("/api/oncall/alerts/{id}/snooze", post(snooze_alert))
        .route("/api/oncall/alerts/{id}/escalate", post(escalate_alert))
        // Silences
        .route("/api/oncall/silences", get(list_silences))
        .route("/api/oncall/silences", post(create_silence))
        .route("/api/oncall/silences/{id}", delete(delete_silence))
        // Webhooks
        .route("/api/oncall/webhooks/prometheus", post(webhook_prometheus))
        .route("/api/oncall/webhooks/generic", post(webhook_generic))
        // Stats
        .route("/api/oncall/stats", get(stats))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-oncall",
        "status": "ok",
        "upstream": "Grafana OnCall"
    }))
}

// Teams
async fn list_teams(AxumState(store): AxumState<Arc<OnCallStore>>) -> Json<serde_json::Value> {
    let teams: Vec<Team> = store.teams.read().await.values().cloned().collect();
    Json(serde_json::json!({ "teams": teams }))
}

async fn create_team(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Json(req): Json<CreateTeamRequest>,
) -> Json<serde_json::Value> {
    let team = Team {
        id: Uuid::new_v4(),
        name: req.name,
        slug: req.slug,
        description: None,
        members: req.members.unwrap_or_default(),
        timezone: req.timezone,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let id = team.id;
    store.teams.write().await.insert(id, team.clone());
    Json(serde_json::to_value(&team).unwrap_or_default())
}

async fn get_team(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    store
        .teams
        .read()
        .await
        .get(&id)
        .map(|t| Json(serde_json::to_value(t).unwrap_or_default()))
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "team not found"))
}

async fn update_team(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateTeamRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut teams = store.teams.write().await;
    let team = teams
        .get_mut(&id)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "team not found"))?;
    team.name = req.name;
    team.slug = req.slug;
    team.timezone = req.timezone;
    team.members = req.members.unwrap_or_default();
    team.updated_at = Utc::now();
    Ok(Json(serde_json::to_value(team.clone()).unwrap_or_default()))
}

async fn delete_team(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if store.teams.write().await.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn team_current_oncall(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let teams = store.teams.read().await;
    let _ = teams
        .get(&id)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "team not found"))?;
    drop(teams);

    let schedules: Vec<Schedule> = store
        .schedules
        .read()
        .await
        .values()
        .filter(|s| s.team_id == id)
        .cloned()
        .collect();

    let rotations: Vec<Rotation> = store.rotations.read().await.values().cloned().collect();
    let overrides: Vec<ShiftOverride> = store.overrides.read().await.values().cloned().collect();

    let mut result = Vec::new();
    for schedule in schedules {
        if let Some(assignment) =
            crate::engine::current_oncall(&schedule, &rotations, &overrides, Utc::now())
        {
            result.push(assignment);
        }
    }

    Ok(Json(serde_json::json!({ "oncall": result })))
}

async fn team_upcoming_shifts(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
    Query(q): Query<UpcomingQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let teams = store.teams.read().await;
    let _ = teams
        .get(&id)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "team not found"))?;
    drop(teams);

    let schedules: Vec<Schedule> = store
        .schedules
        .read()
        .await
        .values()
        .filter(|s| s.team_id == id)
        .cloned()
        .collect();

    let rotations: Vec<Rotation> = store.rotations.read().await.values().cloned().collect();
    let horizon = q.days.unwrap_or(7);

    let mut shifts = Vec::new();
    for schedule in schedules {
        shifts.extend(crate::engine::upcoming_shifts(
            &schedule, &rotations, horizon,
        ));
    }

    shifts.sort_by_key(|s| s.start);
    Ok(Json(serde_json::json!({ "shifts": shifts })))
}

// Users
async fn list_users(AxumState(store): AxumState<Arc<OnCallStore>>) -> Json<serde_json::Value> {
    let users: Vec<User> = store.users.read().await.values().cloned().collect();
    Json(serde_json::json!({ "users": users }))
}

async fn create_user(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Json(req): Json<CreateUserRequest>,
) -> Json<serde_json::Value> {
    let user = User {
        id: Uuid::new_v4(),
        username: req.username,
        email: req.email,
        display_name: req.display_name,
        timezone: req.timezone,
        phone: req.phone,
        slack_id: req.slack_id,
        active: true,
    };
    let id = user.id;
    store.users.write().await.insert(id, user.clone());
    Json(serde_json::to_value(&user).unwrap_or_default())
}

async fn get_user(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    store
        .users
        .read()
        .await
        .get(&id)
        .map(|u| Json(serde_json::to_value(u).unwrap_or_default()))
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "user not found"))
}

// Schedules
async fn list_schedules(AxumState(store): AxumState<Arc<OnCallStore>>) -> Json<serde_json::Value> {
    let schedules: Vec<Schedule> = store.schedules.read().await.values().cloned().collect();
    Json(serde_json::json!({ "schedules": schedules }))
}

async fn create_schedule(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Json(req): Json<CreateScheduleRequest>,
) -> Json<serde_json::Value> {
    let schedule = Schedule {
        id: Uuid::new_v4(),
        team_id: req.team_id,
        name: req.name,
        description: req.description,
        timezone: req.timezone,
        schedule_type: req.schedule_type,
        created_at: Utc::now(),
    };
    let id = schedule.id;
    store.schedules.write().await.insert(id, schedule.clone());
    Json(serde_json::to_value(&schedule).unwrap_or_default())
}

async fn get_schedule(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    store
        .schedules
        .read()
        .await
        .get(&id)
        .map(|s| Json(serde_json::to_value(s).unwrap_or_default()))
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "schedule not found"))
}

async fn schedule_calendar(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
    Query(_q): Query<CalendarQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let schedules = store.schedules.read().await;
    let schedule = schedules
        .get(&id)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "schedule not found"))?;

    let rotations: Vec<Rotation> = store.rotations.read().await.values().cloned().collect();
    let shifts = crate::engine::upcoming_shifts(schedule, &rotations, 30);

    Ok(Json(serde_json::json!({ "shifts": shifts })))
}

async fn delete_schedule(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if store.schedules.write().await.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// Rotations
async fn create_rotation(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(_schedule_id): Path<Uuid>,
    Json(req): Json<CreateRotationRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let rotation = Rotation {
        id: Uuid::new_v4(),
        schedule_id: req.schedule_id,
        name: req.name,
        users: req.users,
        start_date: req.start_date,
        rotation_type: req.rotation_type,
        handoff_hour: req.handoff_hour,
        handoff_minute: req.handoff_minute,
        shift_duration_hours: req.shift_duration_hours,
        active: true,
    };

    if let Err(e) = crate::engine::validate_rotation(&rotation) {
        return Err(error_response(StatusCode::BAD_REQUEST, &e.to_string()));
    }

    let id = rotation.id;
    store.rotations.write().await.insert(id, rotation.clone());
    Ok(Json(serde_json::to_value(&rotation).unwrap_or_default()))
}

async fn list_rotations(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(schedule_id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let rotations: Vec<Rotation> = store
        .rotations
        .read()
        .await
        .values()
        .filter(|r| r.schedule_id == schedule_id)
        .cloned()
        .collect();
    Json(serde_json::json!({ "rotations": rotations }))
}

async fn delete_rotation(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if store.rotations.write().await.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// Overrides
async fn create_override(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(_schedule_id): Path<Uuid>,
    Json(req): Json<CreateOverrideRequest>,
) -> Json<serde_json::Value> {
    let override_shift = ShiftOverride {
        id: Uuid::new_v4(),
        schedule_id: req.schedule_id,
        user: req.user,
        start: req.start,
        end: req.end,
        reason: req.reason,
    };
    let id = override_shift.id;
    store
        .overrides
        .write()
        .await
        .insert(id, override_shift.clone());
    Json(serde_json::to_value(&override_shift).unwrap_or_default())
}

async fn list_overrides(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(schedule_id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let overrides: Vec<ShiftOverride> = store
        .overrides
        .read()
        .await
        .values()
        .filter(|o| o.schedule_id == schedule_id)
        .cloned()
        .collect();
    Json(serde_json::json!({ "overrides": overrides }))
}

async fn delete_override(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if store.overrides.write().await.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// Escalation Policies
async fn list_policies(AxumState(store): AxumState<Arc<OnCallStore>>) -> Json<serde_json::Value> {
    let policies: Vec<EscalationPolicy> = store.policies.read().await.values().cloned().collect();
    Json(serde_json::json!({ "policies": policies }))
}

async fn create_policy(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Json(req): Json<CreateEscalationPolicyRequest>,
) -> Json<serde_json::Value> {
    let policy = EscalationPolicy {
        id: Uuid::new_v4(),
        team_id: req.team_id,
        name: req.name,
        steps: req.steps,
        created_at: Utc::now(),
    };
    let id = policy.id;
    store.policies.write().await.insert(id, policy.clone());
    Json(serde_json::to_value(&policy).unwrap_or_default())
}

async fn get_policy(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    store
        .policies
        .read()
        .await
        .get(&id)
        .map(|p| Json(serde_json::to_value(p).unwrap_or_default()))
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "policy not found"))
}

async fn delete_policy(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if store.policies.write().await.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// Alerts
async fn list_alerts(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Query(q): Query<AlertQuery>,
) -> Json<serde_json::Value> {
    let alerts = store.alerts.read().await;
    let mut list: Vec<Alert> = alerts.values().cloned().collect();

    if let Some(ref state_str) = q.state {
        list.retain(|a| {
            let state_name = format!("{:?}", a.state);
            state_name.to_lowercase() == state_str.to_lowercase()
        });
    }
    if let Some(team_id) = q.team {
        list.retain(|a| a.team_id == team_id);
    }

    Json(serde_json::json!({ "alerts": list }))
}

async fn create_alert(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Json(req): Json<CreateAlertRequest>,
) -> Json<serde_json::Value> {
    let alert = Alert {
        id: Uuid::new_v4(),
        team_id: req.team_id,
        title: req.title,
        severity: req.severity,
        source: req.source,
        fingerprint: req.fingerprint,
        state: AlertState::Firing,
        labels: req.labels,
        annotations: req.annotations,
        created_at: Utc::now(),
        ack_at: None,
        ack_by: None,
        resolved_at: None,
        escalation_policy_id: req.escalation_policy_id,
        current_escalation_step: 0,
    };
    let id = alert.id;
    store.alerts.write().await.insert(id, alert.clone());
    Json(serde_json::to_value(&alert).unwrap_or_default())
}

async fn get_alert(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    store
        .alerts
        .read()
        .await
        .get(&id)
        .map(|a| Json(serde_json::to_value(a).unwrap_or_default()))
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "alert not found"))
}

async fn ack_alert(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AckRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut alerts = store.alerts.write().await;
    let alert = alerts
        .get_mut(&id)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "alert not found"))?;

    if alert.state == AlertState::Acknowledged {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "alert already acknowledged",
        ));
    }

    alert.state = AlertState::Acknowledged;
    alert.ack_at = Some(Utc::now());
    alert.ack_by = Some(req.user);

    Ok(Json(
        serde_json::to_value(alert.clone()).unwrap_or_default(),
    ))
}

async fn resolve_alert(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut alerts = store.alerts.write().await;
    let alert = alerts
        .get_mut(&id)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "alert not found"))?;

    alert.state = AlertState::Resolved;
    alert.resolved_at = Some(Utc::now());

    Ok(Json(
        serde_json::to_value(alert.clone()).unwrap_or_default(),
    ))
}

async fn snooze_alert(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
    Json(_req): Json<SnoozeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut alerts = store.alerts.write().await;
    let alert = alerts
        .get_mut(&id)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "alert not found"))?;

    alert.state = AlertState::Silenced;

    Ok(Json(
        serde_json::to_value(alert.clone()).unwrap_or_default(),
    ))
}

async fn escalate_alert(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut alerts = store.alerts.write().await;
    let alert = alerts
        .get_mut(&id)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "alert not found"))?;

    alert.current_escalation_step = alert.current_escalation_step.saturating_add(1);

    Ok(Json(
        serde_json::to_value(alert.clone()).unwrap_or_default(),
    ))
}

// Silences
async fn list_silences(AxumState(store): AxumState<Arc<OnCallStore>>) -> Json<serde_json::Value> {
    let silences: Vec<Silence> = store.silences.read().await.values().cloned().collect();
    Json(serde_json::json!({ "silences": silences }))
}

async fn create_silence(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Json(req): Json<CreateSilenceRequest>,
) -> Json<serde_json::Value> {
    let silence = Silence {
        id: Uuid::new_v4(),
        team_id: req.team_id,
        matcher: req.matcher,
        start: req.start,
        end: req.end,
        created_by: req.created_by,
        reason: req.reason,
    };
    let id = silence.id;
    store.silences.write().await.insert(id, silence.clone());
    Json(serde_json::to_value(&silence).unwrap_or_default())
}

async fn delete_silence(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    if store.silences.write().await.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// Webhooks
async fn webhook_prometheus(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Json(payload): Json<WebhookPayload>,
) -> StatusCode {
    let alerts = store.alerts.read().await;
    if crate::engine::dedupe_fingerprint(
        &payload.fingerprint,
        &alerts.values().cloned().collect::<Vec<_>>(),
    )
    .is_some()
    {
        return StatusCode::CONFLICT;
    }
    drop(alerts);

    let alert = Alert {
        id: Uuid::new_v4(),
        team_id: Uuid::nil(),
        title: payload.title,
        severity: match payload.severity.to_lowercase().as_str() {
            "critical" => Severity::Critical,
            "high" => Severity::High,
            "medium" => Severity::Medium,
            "low" => Severity::Low,
            _ => Severity::Info,
        },
        source: payload.source,
        fingerprint: payload.fingerprint.clone(),
        state: AlertState::Firing,
        labels: payload.labels,
        annotations: payload.annotations,
        created_at: Utc::now(),
        ack_at: None,
        ack_by: None,
        resolved_at: None,
        escalation_policy_id: None,
        current_escalation_step: 0,
    };

    let alert_id = alert.id;
    store.alerts.write().await.insert(alert_id, alert);
    StatusCode::CREATED
}

async fn webhook_generic(
    AxumState(store): AxumState<Arc<OnCallStore>>,
    Json(payload): Json<WebhookPayload>,
) -> StatusCode {
    let alerts = store.alerts.read().await;
    if crate::engine::dedupe_fingerprint(
        &payload.fingerprint,
        &alerts.values().cloned().collect::<Vec<_>>(),
    )
    .is_some()
    {
        return StatusCode::CONFLICT;
    }
    drop(alerts);

    let alert = Alert {
        id: Uuid::new_v4(),
        team_id: Uuid::nil(),
        title: payload.title,
        severity: match payload.severity.to_lowercase().as_str() {
            "critical" => Severity::Critical,
            "high" => Severity::High,
            "medium" => Severity::Medium,
            "low" => Severity::Low,
            _ => Severity::Info,
        },
        source: payload.source,
        fingerprint: payload.fingerprint,
        state: AlertState::Firing,
        labels: payload.labels,
        annotations: payload.annotations,
        created_at: Utc::now(),
        ack_at: None,
        ack_by: None,
        resolved_at: None,
        escalation_policy_id: None,
        current_escalation_step: 0,
    };

    let alert_id = alert.id;
    store.alerts.write().await.insert(alert_id, alert);
    StatusCode::CREATED
}

// Stats
async fn stats(AxumState(store): AxumState<Arc<OnCallStore>>) -> Json<serde_json::Value> {
    let alerts = store.alerts.read().await;
    let total_alerts = alerts.len() as u64;
    let firing = alerts
        .iter()
        .filter(|a| a.1.state == AlertState::Firing)
        .count() as u64;
    let acknowledged = alerts
        .iter()
        .filter(|a| a.1.state == AlertState::Acknowledged)
        .count() as u64;
    let resolved = alerts
        .iter()
        .filter(|a| a.1.state == AlertState::Resolved)
        .count() as u64;

    let teams_count = store.teams.read().await.len() as u64;
    let schedules_count = store.schedules.read().await.len() as u64;

    Json(serde_json::json!({
        "teams": teams_count,
        "schedules": schedules_count,
        "alerts": {
            "total": total_alerts,
            "firing": firing,
            "acknowledged": acknowledged,
            "resolved": resolved,
        }
    }))
}
