//! HTTP route handlers — Unleash Client API, Admin API, and legacy CAVE API.
//!
//! ## Endpoint map
//!
//! ### Unleash Client API (SDK-facing)
//! - `GET  /api/client/features`         — feature toggle snapshot for SDK polling
//! - `POST /api/client/register`         — SDK client registration
//! - `POST /api/client/metrics`          — SDK usage metrics ingestion
//!
//! ### Unleash Admin API (dashboard / automation)
//! - `GET  /api/admin/features`           — list all non-archived toggles
//! - `POST /api/admin/features`           — create a toggle
//! - `GET  /api/admin/features/:name`     — get a single toggle
//! - `PUT  /api/admin/features/:name`     — update a toggle
//! - `DELETE /api/admin/features/:name`   — archive a toggle
//! - `POST /api/admin/features/:name/toggle/on`  — enable toggle
//! - `POST /api/admin/features/:name/toggle/off` — disable toggle
//! - `POST /api/admin/features/:name/strategies` — add a strategy
//! - `GET  /api/admin/projects`           — list projects
//! - `GET  /api/admin/projects/:id`       — get project
//! - `GET  /api/admin/strategies`         — list built-in strategy definitions
//! - `GET  /api/admin/events`             — audit log
//! - `GET  /api/admin/metrics/feature-toggles` — toggle usage metrics
//!
//! ### Legacy CAVE API
//! - `GET  /api/flags`           — list legacy flags
//! - `POST /api/flags`           — create legacy flag
//! - `POST /api/flags/evaluate`  — evaluate flags for a context
//! - `GET  /api/flags/health`    — health check

use crate::models::*;
use crate::{FlagsState, MetricEntry};
use axum::{
<<<<<<< HEAD
    extract::{Path, State},
=======
    extract::State,
>>>>>>> claude/bold-mahavira
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
<<<<<<< HEAD
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

// ================================================================
// Router factory
// ================================================================
=======
use cave_db::StorageExt;
use std::sync::Arc;
use uuid::Uuid;

const COLLECTION: &str = "flags";
>>>>>>> claude/bold-mahavira

pub fn create_router(state: Arc<FlagsState>) -> Router {
    Router::new()
        // ── Client API ──────────────────────────────────────────
        .route("/api/client/features", get(client_features))
        .route("/api/client/register", post(client_register))
        .route("/api/client/metrics", post(client_metrics))
        // ── Admin API ───────────────────────────────────────────
        .route(
            "/api/admin/features",
            get(admin_list_features).post(admin_create_feature),
        )
        .route(
            "/api/admin/features/:name",
            get(admin_get_feature)
                .put(admin_update_feature)
                .delete(admin_archive_feature),
        )
        .route(
            "/api/admin/features/:name/toggle/on",
            post(admin_toggle_on),
        )
        .route(
            "/api/admin/features/:name/toggle/off",
            post(admin_toggle_off),
        )
        .route(
            "/api/admin/features/:name/strategies",
            post(admin_add_strategy),
        )
        .route("/api/admin/projects", get(admin_list_projects))
        .route("/api/admin/projects/:id", get(admin_get_project))
        .route("/api/admin/strategies", get(admin_list_strategies))
        .route("/api/admin/events", get(admin_list_events))
        .route(
            "/api/admin/metrics/feature-toggles",
            get(admin_metrics),
        )
        // ── Legacy CAVE API ─────────────────────────────────────
        .route("/api/flags", get(list_flags).post(create_flag))
        .route("/api/flags/evaluate", post(evaluate))
        .route("/api/flags/health", get(health))
        .with_state(state)
}

<<<<<<< HEAD
// ================================================================
// Client API handlers
// ================================================================

/// GET /api/client/features
///
/// Returns all active (non-archived) toggles formatted for SDK polling.
/// Compatible with Unleash client SDK `/api/client/features` response shape.
async fn client_features(State(state): State<Arc<FlagsState>>) -> Json<serde_json::Value> {
    let features = state.features.read().await;
    let segments = state.segments.read().await;

    let toggle_list: Vec<serde_json::Value> = features
        .values()
        .filter(|t| !t.archived)
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "enabled": t.enabled,
                "project": t.project,
                "stale": t.stale,
                "type": t.toggle_type,
                "strategies": t.strategies,
                "variants": t.variants,
                "impressionData": t.impression_data,
            })
        })
        .collect();

    let seg_list: Vec<serde_json::Value> = segments
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "name": s.name,
                "constraints": s.constraints,
            })
        })
        .collect();

    Json(serde_json::json!({
        "version": 2,
        "features": toggle_list,
        "segments": seg_list,
    }))
}

/// POST /api/client/register
///
/// Client SDK registration endpoint — records the SDK instance and its
/// supported strategies.  Returns 202 Accepted.
async fn client_register(
    State(state): State<Arc<FlagsState>>,
    Json(reg): Json<ClientRegistration>,
) -> StatusCode {
    let mut events = state.events.write().await;
    let id = events.len() as i64 + 1;
    events.push(Event {
        id,
        event_type: "client-register".to_string(),
        created_by: reg.app_name.clone(),
        data: Some(serde_json::json!({
            "appName": reg.app_name,
            "instanceId": reg.instance_id,
            "sdkVersion": reg.sdk_version,
            "strategies": reg.strategies,
            "started": reg.started,
            "interval": reg.interval,
        })),
        pre_data: None,
        feature_name: None,
        project: None,
        environment: None,
        tags: vec![],
        created_at: Utc::now(),
    });
    tracing::info!(
        app = %reg.app_name,
        instance = %reg.instance_id,
        "Client SDK registered"
    );
    StatusCode::ACCEPTED
}

/// POST /api/client/metrics
///
/// Ingest usage metrics from a client SDK.  Merges yes/no/variant counts
/// into the in-memory metrics store.  Returns 202 Accepted.
async fn client_metrics(
    State(state): State<Arc<FlagsState>>,
    Json(metrics): Json<ClientMetrics>,
) -> StatusCode {
    let mut store = state.metrics.write().await;
    for (toggle_name, counts) in &metrics.bucket.toggles {
        store
            .entry(toggle_name.clone())
            .and_modify(|e| {
                e.yes += counts.yes;
                e.no += counts.no;
                for (variant_name, count) in &counts.variants {
                    *e.variants.entry(variant_name.clone()).or_insert(0) += count;
                }
            })
            .or_insert_with(|| MetricEntry {
                toggle_name: toggle_name.clone(),
                yes: counts.yes,
                no: counts.no,
                variants: counts.variants.clone(),
            });
    }
    tracing::debug!(app = %metrics.app_name, "Metrics ingested");
    StatusCode::ACCEPTED
}

// ================================================================
// Admin API — Feature Toggles
// ================================================================

/// GET /api/admin/features
async fn admin_list_features(State(state): State<Arc<FlagsState>>) -> Json<serde_json::Value> {
    let features = state.features.read().await;
    let list: Vec<&FeatureToggle> = features.values().filter(|t| !t.archived).collect();
    Json(serde_json::json!({ "version": 1, "features": list }))
}

/// POST /api/admin/features
async fn admin_create_feature(
    State(state): State<Arc<FlagsState>>,
    Json(req): Json<CreateToggleRequest>,
) -> Result<(StatusCode, Json<FeatureToggle>), (StatusCode, Json<serde_json::Value>)> {
    let mut features = state.features.write().await;
    if features.contains_key(&req.name) {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "Feature toggle already exists" })),
        ));
    }
    let mut toggle = FeatureToggle::new(req.name.clone(), "admin");
    toggle.description = req.description;
    toggle.project = req.project.unwrap_or_else(|| "default".to_string());
    toggle.toggle_type = req.toggle_type.unwrap_or_else(|| "release".to_string());
    toggle.impression_data = req.impression_data;

    features.insert(toggle.name.clone(), toggle.clone());

    // Record event
    drop(features);
    let mut events = state.events.write().await;
    let id = events.len() as i64 + 1;
    events.push(Event {
        id,
        event_type: "feature-created".to_string(),
        created_by: "admin".to_string(),
        data: serde_json::to_value(&toggle).ok(),
        pre_data: None,
        feature_name: Some(toggle.name.clone()),
        project: Some(toggle.project.clone()),
        environment: None,
        tags: vec![],
        created_at: Utc::now(),
    });

    Ok((StatusCode::CREATED, Json(toggle)))
}

/// GET /api/admin/features/:name
async fn admin_get_feature(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> Result<Json<FeatureToggle>, StatusCode> {
    let features = state.features.read().await;
    features
        .get(&name)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

/// PUT /api/admin/features/:name
async fn admin_update_feature(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
    Json(req): Json<UpdateToggleRequest>,
) -> Result<Json<FeatureToggle>, StatusCode> {
    let mut features = state.features.write().await;
    let toggle = features.get_mut(&name).ok_or(StatusCode::NOT_FOUND)?;

    if let Some(desc) = req.description {
        toggle.description = Some(desc);
    }
    if let Some(stale) = req.stale {
        toggle.stale = stale;
    }
    if let Some(imp) = req.impression_data {
        toggle.impression_data = imp;
    }
    toggle.updated_at = Utc::now();

    Ok(Json(toggle.clone()))
}

/// DELETE /api/admin/features/:name  (archives, not hard-delete)
async fn admin_archive_feature(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut features = state.features.write().await;
    let toggle = features.get_mut(&name).ok_or(StatusCode::NOT_FOUND)?;
    toggle.archived = true;
    toggle.updated_at = Utc::now();
    Ok(Json(serde_json::json!({ "archived": name })))
}

/// POST /api/admin/features/:name/toggle/on
async fn admin_toggle_on(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> Result<Json<FeatureToggle>, StatusCode> {
    let mut features = state.features.write().await;
    let toggle = features.get_mut(&name).ok_or(StatusCode::NOT_FOUND)?;
    toggle.enabled = true;
    toggle.updated_at = Utc::now();
    Ok(Json(toggle.clone()))
}

/// POST /api/admin/features/:name/toggle/off
async fn admin_toggle_off(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> Result<Json<FeatureToggle>, StatusCode> {
    let mut features = state.features.write().await;
    let toggle = features.get_mut(&name).ok_or(StatusCode::NOT_FOUND)?;
    toggle.enabled = false;
    toggle.updated_at = Utc::now();
    Ok(Json(toggle.clone()))
}

/// POST /api/admin/features/:name/strategies
async fn admin_add_strategy(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
    Json(req): Json<AddStrategyRequest>,
) -> Result<Json<FeatureToggle>, StatusCode> {
    let mut features = state.features.write().await;
    let toggle = features.get_mut(&name).ok_or(StatusCode::NOT_FOUND)?;
    toggle.strategies.push(StrategyConfig {
        name: req.name,
        parameters: req.parameters,
        constraints: req.constraints,
        segments: req.segments,
    });
    toggle.updated_at = Utc::now();
    Ok(Json(toggle.clone()))
}

// ================================================================
// Admin API — Projects
// ================================================================

/// GET /api/admin/projects
async fn admin_list_projects(State(state): State<Arc<FlagsState>>) -> Json<serde_json::Value> {
    let projects = state.projects.read().await;
    let features = state.features.read().await;

    // Update feature counts dynamically
    let enriched: Vec<serde_json::Value> = projects
        .iter()
        .map(|p| {
            let count = features
                .values()
                .filter(|f| f.project == p.id && !f.archived)
                .count();
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "description": p.description,
                "createdAt": p.created_at,
                "health": p.health,
                "featureCount": count,
                "memberCount": p.member_count,
            })
        })
        .collect();

    Json(serde_json::json!({ "version": 1, "projects": enriched }))
}

/// GET /api/admin/projects/:id
async fn admin_get_project(
    State(state): State<Arc<FlagsState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let projects = state.projects.read().await;
    let project = projects
        .iter()
        .find(|p| p.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let features = state.features.read().await;
    let toggles: Vec<&FeatureToggle> = features
        .values()
        .filter(|f| f.project == id && !f.archived)
        .collect();

    Ok(Json(serde_json::json!({
        "id": project.id,
        "name": project.name,
        "description": project.description,
        "createdAt": project.created_at,
        "health": project.health,
        "features": toggles,
        "members": project.member_count,
    })))
}

// ================================================================
// Admin API — Strategy definitions (static catalogue)
// ================================================================

/// GET /api/admin/strategies
///
/// Returns Unleash-compatible built-in strategy definitions.
async fn admin_list_strategies() -> Json<serde_json::Value> {
    let strategies = serde_json::json!([
        {
            "name": "default",
            "displayName": "Standard strategy",
            "description": "Activate the feature for all users.",
            "editable": false,
            "deprecated": false,
            "parameters": []
        },
        {
            "name": "userWithId",
            "displayName": "UserIDs",
            "description": "Enable the feature for a specific set of userIds.",
            "editable": false,
            "deprecated": false,
            "parameters": [
                {
                    "name": "userIds",
                    "type": "list",
                    "description": "Comma-separated list of user IDs that should have the feature activated.",
                    "required": true
                }
            ]
        },
        {
            "name": "gradualRolloutUserId",
            "displayName": "Gradual rollout with user ID",
            "description": "Gradually activate feature toggle. Stickiness based on user ID.",
            "editable": false,
            "deprecated": true,
            "parameters": [
                { "name": "percentage", "type": "percentage", "description": "% of users to activate.", "required": true },
                { "name": "groupId", "type": "string", "description": "Used to calculate hash.", "required": true }
            ]
        },
        {
            "name": "gradualRolloutSessionId",
            "displayName": "Gradual rollout with session ID",
            "description": "Gradually activate feature toggle. Stickiness based on session ID.",
            "editable": false,
            "deprecated": true,
            "parameters": [
                { "name": "percentage", "type": "percentage", "description": "% of sessions to activate.", "required": true },
                { "name": "groupId", "type": "string", "description": "Used to calculate hash.", "required": true }
            ]
        },
        {
            "name": "gradualRolloutRandom",
            "displayName": "Gradual rollout random",
            "description": "Randomly activate the feature toggle. No stickiness.",
            "editable": false,
            "deprecated": true,
            "parameters": [
                { "name": "percentage", "type": "percentage", "description": "% of requests to activate.", "required": true }
            ]
        },
        {
            "name": "flexibleRollout",
            "displayName": "Gradual rollout",
            "description": "Roll out to a percentage of users with configurable stickiness.",
            "editable": false,
            "deprecated": false,
            "parameters": [
                { "name": "rollout", "type": "percentage", "description": "% of users to activate.", "required": true },
                { "name": "stickiness", "type": "string", "description": "Stickiness field: default, userId, sessionId, random.", "required": true },
                { "name": "groupId", "type": "string", "description": "Group identifier for hash calculation.", "required": true }
            ]
        },
        {
            "name": "applicationHostname",
            "displayName": "Hosts",
            "description": "Activate for applications running on specific hostnames.",
            "editable": false,
            "deprecated": false,
            "parameters": [
                { "name": "hostNames", "type": "list", "description": "Comma-separated list of hostnames.", "required": true }
            ]
        },
        {
            "name": "remoteAddress",
            "displayName": "IPs",
            "description": "Activate for clients with specific remote addresses or CIDR ranges.",
            "editable": false,
            "deprecated": false,
            "parameters": [
                { "name": "IPs", "type": "list", "description": "Comma-separated list of IPs or CIDR ranges.", "required": true }
            ]
        }
    ]);

    Json(serde_json::json!({ "version": 1, "strategies": strategies }))
}

// ================================================================
// Admin API — Events / Audit Log
// ================================================================

/// GET /api/admin/events
async fn admin_list_events(State(state): State<Arc<FlagsState>>) -> Json<serde_json::Value> {
    let events = state.events.read().await;
    // Return most-recent first
    let mut list: Vec<&Event> = events.iter().collect();
    list.reverse();
    Json(serde_json::json!({ "version": 1, "events": list }))
}

// ================================================================
// Admin API — Metrics
// ================================================================

/// GET /api/admin/metrics/feature-toggles
async fn admin_metrics(State(state): State<Arc<FlagsState>>) -> Json<serde_json::Value> {
    let metrics = state.metrics.read().await;
    let toggle_metrics: Vec<serde_json::Value> = metrics
        .values()
        .map(|m| {
            serde_json::json!({
                "toggleName": m.toggle_name,
                "yes": m.yes,
                "no": m.no,
                "variants": m.variants,
            })
        })
        .collect();
    Json(serde_json::json!({ "version": 1, "maturity": "stable", "toggles": toggle_metrics }))
}

// ================================================================
// Legacy CAVE API handlers
// ================================================================

/// GET /api/flags — list all legacy flags
async fn list_flags(State(_state): State<Arc<FlagsState>>) -> Json<Vec<FeatureFlag>> {
    Json(vec![])
=======
/// GET /api/flags — list all flags
async fn list_flags(
    State(state): State<Arc<FlagsState>>,
) -> Result<Json<Vec<FeatureFlag>>, StatusCode> {
    let flags = state
        .storage
        .list::<FeatureFlag>(COLLECTION)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(flags))
>>>>>>> claude/bold-mahavira
}

/// POST /api/flags — create a new legacy flag
async fn create_flag(
    State(state): State<Arc<FlagsState>>,
    Json(req): Json<CreateFlagRequest>,
<<<<<<< HEAD
) -> Json<FeatureFlag> {
    let now = Utc::now();
    Json(FeatureFlag {
=======
) -> Result<Json<FeatureFlag>, StatusCode> {
    let now = chrono::Utc::now();
    let flag = FeatureFlag {
>>>>>>> claude/bold-mahavira
        id: Uuid::new_v4(),
        name: req.name,
        description: req.description,
        enabled: true,
        flag_type: req.flag_type,
        strategy: req.strategy,
        environments: req.environments,
        tenant_id: req.tenant_id,
        kill_switch: false,
        created_at: now,
        updated_at: now,
<<<<<<< HEAD
        created_by: Uuid::new_v4(),
    })
=======
        created_by: Uuid::new_v4(), // TODO: extract from CaveIdentity
    };

    state
        .storage
        .put::<FeatureFlag>(COLLECTION, &flag.id.to_string(), &flag)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(flag))
>>>>>>> claude/bold-mahavira
}

/// POST /api/flags/evaluate — evaluate all flags for a context
async fn evaluate(
    State(state): State<Arc<FlagsState>>,
    Json(req): Json<EvaluateRequest>,
<<<<<<< HEAD
) -> Json<EvaluateResponse> {
    let flags: Vec<FeatureFlag> = vec![];
=======
) -> Result<Json<EvaluateResponse>, StatusCode> {
    let flags = state
        .storage
        .list::<FeatureFlag>(COLLECTION)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

>>>>>>> claude/bold-mahavira
    let evaluations = crate::engine::evaluate_flags(&flags, &req.context);
    Ok(Json(EvaluateResponse { flags: evaluations }))
}

/// GET /api/flags/health
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-flags",
        "status": "ok",
        "upstream": "unleash",
        "upstream_tracked_version": "6.x",
        "strategies": [
            "default", "userWithId", "gradualRolloutUserId",
            "gradualRolloutSessionId", "gradualRolloutRandom",
            "flexibleRollout", "applicationHostname", "remoteAddress"
        ]
    }))
}
