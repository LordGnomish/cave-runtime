// SPDX-License-Identifier: AGPL-3.0-or-later
//! Unleash-compatible HTTP routes for the flags module.
//!
//! Implements:
//!  - Client SDK API  (/api/client/*)
//!  - Frontend API    (/api/frontend/*)
//!  - Admin API       (/api/admin/*)

use crate::engine::{evaluate_all, evaluate_flag};
use crate::models::*;
use crate::FlagsState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<FlagsState>) -> Router {
    Router::new()
        // ── Client SDK ────────────────────────────────────────────────────────
        .route("/api/client/features", get(client_features))
        .route("/api/client/features/{name}", get(client_feature_single))
        .route("/api/client/metrics", post(client_metrics))
        .route("/api/client/register", post(client_register))
        // ── Frontend ─────────────────────────────────────────────────────────
        .route("/api/frontend", get(frontend_toggles_get).post(frontend_toggles_post))
        .route("/api/frontend/features", get(frontend_features_list))
        .route("/api/frontend/features/{name}", get(frontend_feature_single))
        // ── Admin: features ──────────────────────────────────────────────────
        .route(
            "/api/admin/features",
            get(admin_list_features).post(admin_create_feature),
        )
        .route(
            "/api/admin/features/{name}",
            get(admin_get_feature)
                .put(admin_update_feature)
                .delete(admin_archive_feature),
        )
        .route(
            "/api/admin/features/{name}/environments/{env}/on",
            post(admin_enable_feature_env),
        )
        .route(
            "/api/admin/features/{name}/environments/{env}/off",
            post(admin_disable_feature_env),
        )
        .route(
            "/api/admin/features/{name}/environments/{env}/strategies",
            get(admin_list_strategies).post(admin_add_strategy),
        )
        .route(
            "/api/admin/features/{name}/environments/{env}/strategies/{sid}",
            put(admin_update_strategy).delete(admin_delete_strategy),
        )
        .route(
            "/api/admin/features/{name}/variants",
            get(admin_get_variants).put(admin_set_variants),
        )
        .route(
            "/api/admin/features/{name}/tags",
            get(admin_get_tags).post(admin_add_tag),
        )
        // ── Admin: projects ──────────────────────────────────────────────────
        .route(
            "/api/admin/projects",
            get(admin_list_projects).post(admin_create_project),
        )
        .route(
            "/api/admin/projects/{id}",
            get(admin_get_project).put(admin_update_project).delete(admin_delete_project),
        )
        // ── Admin: environments ──────────────────────────────────────────────
        .route("/api/admin/environments", get(admin_list_environments))
        .route("/api/admin/environments/{name}", get(admin_get_environment))
        // ── Admin: strategies ────────────────────────────────────────────────
        .route("/api/admin/strategies", get(admin_list_strategy_definitions))
        // ── Admin: segments ──────────────────────────────────────────────────
        .route(
            "/api/admin/segments",
            get(admin_list_segments).post(admin_create_segment),
        )
        .route(
            "/api/admin/segments/{id}",
            get(admin_get_segment).put(admin_update_segment).delete(admin_delete_segment),
        )
        // ── Admin: context fields ────────────────────────────────────────────
        .route("/api/admin/context", get(admin_list_context_fields))
        // ── Admin: API tokens ────────────────────────────────────────────────
        .route(
            "/api/admin/api-tokens",
            get(admin_list_tokens).post(admin_create_token),
        )
        .route("/api/admin/api-tokens/{secret}", delete(admin_delete_token))
        // ── Admin: banners ───────────────────────────────────────────────────
        .route(
            "/api/admin/banners",
            get(admin_list_banners).post(admin_create_banner),
        )
        .route(
            "/api/admin/banners/{id}",
            put(admin_update_banner).delete(admin_delete_banner),
        )
        // ── Admin: change requests ───────────────────────────────────────────
        .route(
            "/api/admin/projects/{project}/change-requests",
            get(admin_list_change_requests).post(admin_create_change_request),
        )
        .route(
            "/api/admin/projects/{project}/change-requests/{id}/approve",
            post(admin_approve_change_request),
        )
        .route(
            "/api/admin/projects/{project}/change-requests/{id}/apply",
            post(admin_apply_change_request),
        )
        // ── Health ───────────────────────────────────────────────────────────
        .route("/api/flags/health", get(health))
        .with_state(state)
}

// ── Health ─────────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-flags",
        "status": "ok",
        "upstream": "unleash",
        "upstream_tracked_version": "6.x"
    }))
}

// ── Client SDK ─────────────────────────────────────────────────────────────────

/// GET /api/client/features — returns all features for the client SDK.
async fn client_features(State(state): State<Arc<FlagsState>>) -> Json<ClientFeaturesResponse> {
    let cache = state.cache.read().await;
    let features: Vec<ClientFeature> = cache
        .features
        .iter()
        .map(|f| ClientFeature {
            name: f.name.clone(),
            feature_type: f.feature_type.clone(),
            enabled: f.enabled,
            stale: f.stale,
            strategies: f.strategies.clone(),
            variants: f.variants.clone(),
            impression_data: f.impression_data,
            last_seen_at: f.last_seen_at,
            created_at: f.created_at,
        })
        .collect();
    let segments = cache.segments.clone();
    Json(ClientFeaturesResponse {
        version: 2,
        features,
        segments,
        query: ClientQuery::default(),
    })
}

/// GET /api/client/features/:name
async fn client_feature_single(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> Result<Json<ClientFeature>, StatusCode> {
    let cache = state.cache.read().await;
    let f = cache
        .features
        .iter()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(ClientFeature {
        name: f.name.clone(),
        feature_type: f.feature_type.clone(),
        enabled: f.enabled,
        stale: f.stale,
        strategies: f.strategies.clone(),
        variants: f.variants.clone(),
        impression_data: f.impression_data,
        last_seen_at: f.last_seen_at,
        created_at: f.created_at,
    }))
}

/// POST /api/client/metrics
async fn client_metrics(
    State(_state): State<Arc<FlagsState>>,
    Json(_body): Json<MetricsReport>,
) -> StatusCode {
    // Persisting metrics to DB is a background concern; accept and return 202.
    StatusCode::ACCEPTED
}

/// POST /api/client/register
async fn client_register(
    State(_state): State<Arc<FlagsState>>,
    Json(_body): Json<SdkRegistration>,
) -> StatusCode {
    StatusCode::ACCEPTED
}

// ── Frontend ───────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct ContextQuery {
    #[serde(rename = "userId")]
    user_id: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "remoteAddress")]
    remote_address: Option<String>,
    environment: Option<String>,
    #[serde(rename = "appName")]
    app_name: Option<String>,
}

fn context_from_query(q: ContextQuery) -> UnleashContext {
    UnleashContext {
        user_id: q.user_id,
        session_id: q.session_id,
        remote_address: q.remote_address,
        environment: q.environment,
        app_name: q.app_name,
        current_time: None,
        properties: HashMap::new(),
    }
}

/// GET /api/frontend — evaluate all toggles for the context supplied in query params.
async fn frontend_toggles_get(
    State(state): State<Arc<FlagsState>>,
    Query(q): Query<ContextQuery>,
) -> Json<FrontendFeaturesResponse> {
    let ctx = context_from_query(q);
    evaluate_for_frontend(state, ctx).await
}

/// POST /api/frontend — evaluate all toggles for the context in the request body.
async fn frontend_toggles_post(
    State(state): State<Arc<FlagsState>>,
    Json(ctx): Json<UnleashContext>,
) -> Json<FrontendFeaturesResponse> {
    evaluate_for_frontend(state, ctx).await
}

async fn evaluate_for_frontend(
    state: Arc<FlagsState>,
    ctx: UnleashContext,
) -> Json<FrontendFeaturesResponse> {
    let cache = state.cache.read().await;
    let env = ctx.environment.as_deref().unwrap_or("production");
    let raw = evaluate_all(&cache.features, env, &ctx, &cache.segments);
    let toggles: Vec<FrontendToggle> = cache
        .features
        .iter()
        .zip(raw.into_iter())
        .map(|(flag, (_name, enabled, variant))| FrontendToggle {
            name: flag.name.clone(),
            enabled,
            variant,
            impression_data: flag.impression_data,
        })
        .collect();
    Json(FrontendFeaturesResponse { toggles })
}

/// GET /api/frontend/features — list all feature names visible to the frontend token.
async fn frontend_features_list(State(state): State<Arc<FlagsState>>) -> Json<serde_json::Value> {
    let cache = state.cache.read().await;
    let names: Vec<&str> = cache.features.iter().map(|f| f.name.as_str()).collect();
    Json(serde_json::json!({ "features": names }))
}

/// GET /api/frontend/features/:name
async fn frontend_feature_single(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
    Query(q): Query<ContextQuery>,
) -> Result<Json<FrontendToggle>, StatusCode> {
    let cache = state.cache.read().await;
    let flag = cache
        .features
        .iter()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    let ctx = context_from_query(q);
    let env = ctx.environment.as_deref().unwrap_or("production");
    let seg_map: HashMap<i64, &Segment> = cache.segments.iter().map(|s| (s.id, s)).collect();
    let result = evaluate_flag(flag, env, &ctx, &seg_map);
    Ok(Json(FrontendToggle {
        name: flag.name.clone(),
        enabled: result.enabled,
        variant: result.variant,
        impression_data: flag.impression_data,
    }))
}

// ── Admin: features ────────────────────────────────────────────────────────────

async fn admin_list_features(State(state): State<Arc<FlagsState>>) -> Json<serde_json::Value> {
    let cache = state.cache.read().await;
    Json(serde_json::json!({ "features": cache.features }))
}

async fn admin_create_feature(
    State(state): State<Arc<FlagsState>>,
    Json(req): Json<CreateFeatureRequest>,
) -> (StatusCode, Json<FeatureFlag>) {
    let flag = FeatureFlag {
        name: req.name,
        feature_type: req.feature_type.unwrap_or(FeatureType::Release),
        description: req.description.unwrap_or_default(),
        enabled: true,
        stale: false,
        impression_data: req.impression_data.unwrap_or(false),
        project: "default".to_string(),
        created_at: Utc::now(),
        last_seen_at: None,
        strategies: vec![],
        variants: vec![],
        environments: vec![],
        tags: vec![],
    };
    let mut cache = state.cache.write().await;
    cache.features.push(flag.clone());
    (StatusCode::CREATED, Json(flag))
}

async fn admin_get_feature(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> Result<Json<FeatureFlag>, StatusCode> {
    let cache = state.cache.read().await;
    cache
        .features
        .iter()
        .find(|f| f.name == name)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn admin_update_feature(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
    Json(req): Json<UpdateFeatureRequest>,
) -> Result<Json<FeatureFlag>, StatusCode> {
    let mut cache = state.cache.write().await;
    let flag = cache
        .features
        .iter_mut()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    if let Some(d) = req.description {
        flag.description = d;
    }
    if let Some(t) = req.feature_type {
        flag.feature_type = t;
    }
    if let Some(s) = req.stale {
        flag.stale = s;
    }
    if let Some(i) = req.impression_data {
        flag.impression_data = i;
    }
    Ok(Json(flag.clone()))
}

async fn admin_archive_feature(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> StatusCode {
    let mut cache = state.cache.write().await;
    if let Some(pos) = cache.features.iter().position(|f| f.name == name) {
        cache.features.remove(pos);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn admin_enable_feature_env(
    State(state): State<Arc<FlagsState>>,
    Path((name, env)): Path<(String, String)>,
) -> StatusCode {
    set_feature_env_enabled(state, name, env, true).await
}

async fn admin_disable_feature_env(
    State(state): State<Arc<FlagsState>>,
    Path((name, env)): Path<(String, String)>,
) -> StatusCode {
    set_feature_env_enabled(state, name, env, false).await
}

async fn set_feature_env_enabled(
    state: Arc<FlagsState>,
    name: String,
    env: String,
    enabled: bool,
) -> StatusCode {
    let mut cache = state.cache.write().await;
    let Some(flag) = cache.features.iter_mut().find(|f| f.name == name) else {
        return StatusCode::NOT_FOUND;
    };
    if let Some(fe) = flag.environments.iter_mut().find(|e| e.name == env) {
        fe.enabled = enabled;
    } else {
        flag.environments.push(FeatureEnvironment {
            name: env,
            enabled,
            strategies: vec![],
            variants: vec![],
        });
    }
    StatusCode::OK
}

async fn admin_list_strategies(
    State(state): State<Arc<FlagsState>>,
    Path((name, env)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let cache = state.cache.read().await;
    let flag = cache
        .features
        .iter()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    let strats: Vec<&FeatureStrategy> = flag
        .strategies
        .iter()
        .filter(|s| {
            flag.environments
                .iter()
                .any(|e| e.name == env && e.strategies.iter().any(|es| es.id == s.id))
        })
        .collect();
    Ok(Json(serde_json::json!({ "strategies": strats })))
}

async fn admin_add_strategy(
    State(state): State<Arc<FlagsState>>,
    Path((name, env)): Path<(String, String)>,
    Json(req): Json<AddStrategyRequest>,
) -> Result<(StatusCode, Json<FeatureStrategy>), StatusCode> {
    let strategy = FeatureStrategy {
        id: Uuid::new_v4(),
        name: req.name,
        parameters: req.parameters.unwrap_or_default(),
        constraints: req.constraints.unwrap_or_default(),
        segments: req.segments.unwrap_or_default(),
        sort_order: req.sort_order.unwrap_or(0),
        disabled: false,
        variants: req.variants.unwrap_or_default(),
    };
    let mut cache = state.cache.write().await;
    let flag = cache
        .features
        .iter_mut()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    flag.strategies.push(strategy.clone());
    if let Some(fe) = flag.environments.iter_mut().find(|e| e.name == env) {
        fe.strategies.push(strategy.clone());
    }
    Ok((StatusCode::CREATED, Json(strategy)))
}

async fn admin_update_strategy(
    State(state): State<Arc<FlagsState>>,
    Path((name, _env, sid)): Path<(String, String, Uuid)>,
    Json(req): Json<AddStrategyRequest>,
) -> Result<Json<FeatureStrategy>, StatusCode> {
    let mut cache = state.cache.write().await;
    let flag = cache
        .features
        .iter_mut()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    let s = flag
        .strategies
        .iter_mut()
        .find(|s| s.id == sid)
        .ok_or(StatusCode::NOT_FOUND)?;
    s.name = req.name;
    if let Some(p) = req.parameters {
        s.parameters = p;
    }
    if let Some(c) = req.constraints {
        s.constraints = c;
    }
    if let Some(segs) = req.segments {
        s.segments = segs;
    }
    if let Some(v) = req.variants {
        s.variants = v;
    }
    Ok(Json(s.clone()))
}

async fn admin_delete_strategy(
    State(state): State<Arc<FlagsState>>,
    Path((name, _env, sid)): Path<(String, String, Uuid)>,
) -> StatusCode {
    let mut cache = state.cache.write().await;
    let Some(flag) = cache.features.iter_mut().find(|f| f.name == name) else {
        return StatusCode::NOT_FOUND;
    };
    let before = flag.strategies.len();
    flag.strategies.retain(|s| s.id != sid);
    for fe in flag.environments.iter_mut() {
        fe.strategies.retain(|s| s.id != sid);
    }
    if flag.strategies.len() < before { StatusCode::OK } else { StatusCode::NOT_FOUND }
}

async fn admin_get_variants(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let cache = state.cache.read().await;
    let flag = cache
        .features
        .iter()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::json!({ "variants": flag.variants })))
}

async fn admin_set_variants(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, StatusCode> {
    let variants: Vec<Variant> =
        serde_json::from_value(body["variants"].clone()).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut cache = state.cache.write().await;
    let flag = cache
        .features
        .iter_mut()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    flag.variants = variants;
    Ok(StatusCode::OK)
}

async fn admin_get_tags(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let cache = state.cache.read().await;
    let flag = cache
        .features
        .iter()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::json!({ "tags": flag.tags })))
}

async fn admin_add_tag(
    State(state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
    Json(tag): Json<Tag>,
) -> Result<StatusCode, StatusCode> {
    let mut cache = state.cache.write().await;
    let flag = cache
        .features
        .iter_mut()
        .find(|f| f.name == name)
        .ok_or(StatusCode::NOT_FOUND)?;
    flag.tags.push(tag);
    Ok(StatusCode::CREATED)
}

// ── Admin: projects ────────────────────────────────────────────────────────────

async fn admin_list_projects() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "projects": [{
            "id": "default",
            "name": "Default",
            "description": "",
            "defaultStickiness": "default",
            "mode": "open",
            "members": 0,
            "health": 100,
            "feature_count": 0,
            "created_at": Utc::now(),
            "updated_at": Utc::now()
        }]
    }))
}

async fn admin_create_project(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    Json(body)
}

async fn admin_get_project(Path(id): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "id": id,
        "name": id,
        "description": "",
        "defaultStickiness": "default",
        "mode": "open",
        "members": 0,
        "health": 100,
        "feature_count": 0,
        "created_at": Utc::now(),
        "updated_at": Utc::now()
    }))
}

async fn admin_update_project(
    Path(_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(body)
}

async fn admin_delete_project(Path(_id): Path<String>) -> StatusCode {
    StatusCode::OK
}

// ── Admin: environments ────────────────────────────────────────────────────────

async fn admin_list_environments() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "environments": [
            { "name": "development", "type": "development", "enabled": true, "protected": false, "sortOrder": 1 },
            { "name": "staging",     "type": "staging",     "enabled": true, "protected": false, "sortOrder": 2 },
            { "name": "production",  "type": "production",  "enabled": true, "protected": true,  "sortOrder": 3 }
        ]
    }))
}

async fn admin_get_environment(Path(name): Path<String>) -> Json<serde_json::Value> {
    let env_type = match name.as_str() {
        "development" => "development",
        "staging" => "staging",
        _ => "production",
    };
    Json(serde_json::json!({
        "name": name,
        "type": env_type,
        "enabled": true,
        "protected": name == "production",
        "sortOrder": 0
    }))
}

// ── Admin: strategy definitions ────────────────────────────────────────────────

async fn admin_list_strategy_definitions() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "strategies": built_in_strategies() }))
}

fn built_in_strategies() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "default",
            "displayName": "Standard",
            "description": "A simple on/off toggle",
            "parameters": [],
            "builtIn": true,
            "deprecated": false
        }),
        serde_json::json!({
            "name": "userWithId",
            "displayName": "UserIDs",
            "description": "Enable for a specific list of user IDs",
            "parameters": [
                { "name": "userIds", "type": "list", "description": "Comma-separated list of user IDs", "required": true }
            ],
            "builtIn": true,
            "deprecated": false
        }),
        serde_json::json!({
            "name": "gradualRolloutRandom",
            "displayName": "Gradual Rollout Random",
            "description": "Roll out to a random percentage of users",
            "parameters": [
                { "name": "percentage", "type": "percentage", "description": "Percentage of users", "required": true }
            ],
            "builtIn": true,
            "deprecated": true
        }),
        serde_json::json!({
            "name": "gradualRolloutSessionId",
            "displayName": "Gradual Rollout SessionId",
            "description": "Roll out based on session ID hash",
            "parameters": [
                { "name": "percentage", "type": "percentage", "description": "Percentage of sessions", "required": true },
                { "name": "groupId", "type": "string", "description": "Used for stickiness", "required": false }
            ],
            "builtIn": true,
            "deprecated": true
        }),
        serde_json::json!({
            "name": "gradualRolloutUserId",
            "displayName": "Gradual Rollout UserId",
            "description": "Roll out based on user ID hash",
            "parameters": [
                { "name": "percentage", "type": "percentage", "description": "Percentage of users", "required": true },
                { "name": "groupId", "type": "string", "description": "Used for stickiness", "required": false }
            ],
            "builtIn": true,
            "deprecated": true
        }),
        serde_json::json!({
            "name": "flexibleRollout",
            "displayName": "Gradual Rollout",
            "description": "Flexible rollout supporting all stickiness options",
            "parameters": [
                { "name": "rollout", "type": "percentage", "description": "Percentage to enable", "required": true },
                { "name": "stickiness", "type": "string", "description": "Stickiness type", "required": true },
                { "name": "groupId", "type": "string", "description": "Feature group ID", "required": false }
            ],
            "builtIn": true,
            "deprecated": false
        }),
        serde_json::json!({
            "name": "remoteAddress",
            "displayName": "IPs",
            "description": "Enable for specific IP addresses",
            "parameters": [
                { "name": "IPs", "type": "list", "description": "Comma-separated list of IPs/CIDRs", "required": true }
            ],
            "builtIn": true,
            "deprecated": false
        }),
        serde_json::json!({
            "name": "applicationHostname",
            "displayName": "Hosts",
            "description": "Enable for specific application hostnames",
            "parameters": [
                { "name": "hostNames", "type": "list", "description": "Comma-separated list of hostnames", "required": true }
            ],
            "builtIn": true,
            "deprecated": false
        }),
    ]
}

// ── Admin: segments ────────────────────────────────────────────────────────────

async fn admin_list_segments(State(state): State<Arc<FlagsState>>) -> Json<serde_json::Value> {
    let cache = state.cache.read().await;
    Json(serde_json::json!({ "segments": cache.segments }))
}

async fn admin_create_segment(
    State(state): State<Arc<FlagsState>>,
    Json(req): Json<CreateSegmentRequest>,
) -> (StatusCode, Json<Segment>) {
    let mut cache = state.cache.write().await;
    let id = cache.segments.iter().map(|s| s.id).max().unwrap_or(0) + 1;
    let segment = Segment {
        id,
        name: req.name,
        description: req.description,
        constraints: req.constraints,
        created_at: Utc::now(),
        created_by: None,
        project: req.project,
    };
    cache.segments.push(segment.clone());
    (StatusCode::CREATED, Json(segment))
}

async fn admin_get_segment(
    State(state): State<Arc<FlagsState>>,
    Path(id): Path<i64>,
) -> Result<Json<Segment>, StatusCode> {
    let cache = state.cache.read().await;
    cache
        .segments
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn admin_update_segment(
    State(state): State<Arc<FlagsState>>,
    Path(id): Path<i64>,
    Json(req): Json<CreateSegmentRequest>,
) -> Result<Json<Segment>, StatusCode> {
    let mut cache = state.cache.write().await;
    let seg = cache
        .segments
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    seg.name = req.name;
    seg.description = req.description;
    seg.constraints = req.constraints;
    Ok(Json(seg.clone()))
}

async fn admin_delete_segment(
    State(state): State<Arc<FlagsState>>,
    Path(id): Path<i64>,
) -> StatusCode {
    let mut cache = state.cache.write().await;
    let before = cache.segments.len();
    cache.segments.retain(|s| s.id != id);
    if cache.segments.len() < before { StatusCode::OK } else { StatusCode::NOT_FOUND }
}

// ── Admin: context fields ──────────────────────────────────────────────────────

async fn admin_list_context_fields() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "contextFields": [
            { "name": "userId",        "description": "The user ID",          "stickiness": true,  "legalValues": [] },
            { "name": "sessionId",     "description": "The session ID",       "stickiness": true,  "legalValues": [] },
            { "name": "remoteAddress", "description": "The remote address",   "stickiness": false, "legalValues": [] },
            { "name": "environment",   "description": "The environment",      "stickiness": false, "legalValues": [] },
            { "name": "appName",       "description": "The application name", "stickiness": false, "legalValues": [] },
            { "name": "currentTime",   "description": "The current time",     "stickiness": false, "legalValues": [] }
        ]
    }))
}

// ── Admin: API tokens ──────────────────────────────────────────────────────────

async fn admin_list_tokens() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "tokens": [] }))
}

async fn admin_create_token(
    Json(req): Json<CreateApiTokenRequest>,
) -> (StatusCode, Json<ApiToken>) {
    let secret = format!(
        "{}:{}.*{}",
        req.projects
            .as_ref()
            .and_then(|p| p.first())
            .map(String::as_str)
            .unwrap_or("*"),
        req.environment.as_deref().unwrap_or("*"),
        Uuid::new_v4().simple()
    );
    let token = ApiToken {
        secret,
        username: req.username,
        token_type: req.token_type,
        environment: req.environment,
        project: req
            .projects
            .as_ref()
            .and_then(|p| p.first())
            .cloned(),
        projects: req.projects.unwrap_or_default(),
        created_at: Utc::now(),
        expires_at: req.expires_at,
        seen_at: None,
    };
    (StatusCode::CREATED, Json(token))
}

async fn admin_delete_token(Path(_secret): Path<String>) -> StatusCode {
    StatusCode::OK
}

// ── Admin: banners ─────────────────────────────────────────────────────────────

async fn admin_list_banners() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "banners": [] }))
}

async fn admin_create_banner(
    Json(req): Json<CreateBannerRequest>,
) -> (StatusCode, Json<Banner>) {
    let banner = Banner {
        id: 1,
        message: req.message,
        variant: req.variant,
        link: req.link,
        link_text: req.link_text,
        enabled: req.enabled.unwrap_or(true),
        created_at: Utc::now(),
    };
    (StatusCode::CREATED, Json(banner))
}

async fn admin_update_banner(
    Path(_id): Path<i64>,
    Json(req): Json<CreateBannerRequest>,
) -> Json<Banner> {
    Json(Banner {
        id: _id,
        message: req.message,
        variant: req.variant,
        link: req.link,
        link_text: req.link_text,
        enabled: req.enabled.unwrap_or(true),
        created_at: Utc::now(),
    })
}

async fn admin_delete_banner(Path(_id): Path<i64>) -> StatusCode {
    StatusCode::OK
}

// ── Admin: change requests ─────────────────────────────────────────────────────

async fn admin_list_change_requests(
    Path(project): Path<String>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "changeRequests": [], "project": project }))
}

async fn admin_create_change_request(
    Path(project): Path<String>,
    Json(req): Json<CreateChangeRequestRequest>,
) -> (StatusCode, Json<ChangeRequest>) {
    let cr = ChangeRequest {
        id: 1,
        title: req.title.unwrap_or_else(|| "Change request".to_string()),
        state: ChangeRequestState::Draft,
        project,
        environment: req.environment,
        min_approvals: req.min_approvals.unwrap_or(1),
        approvals: vec![],
        rejections: vec![],
        changes: vec![],
        created_by: "system".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    (StatusCode::CREATED, Json(cr))
}

async fn admin_approve_change_request(
    Path((_project, _id)): Path<(String, i64)>,
) -> StatusCode {
    StatusCode::OK
}

async fn admin_apply_change_request(
    Path((_project, _id)): Path<(String, i64)>,
) -> StatusCode {
    StatusCode::OK
}
