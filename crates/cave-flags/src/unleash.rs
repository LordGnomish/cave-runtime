// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Unleash-compatible HTTP API for cave-flags.
//!
//! Adds the exact API surface that Unleash server exposes so that any
//! Unleash client SDK (Go, Node, Java, Python, …) can point to cave-flags
//! without modification.
//!
//! ## Endpoints
//! - GET  /api/client/features   — client SDK polling endpoint
//! - GET  /api/admin/features    — admin UI / API list
//! - POST /api/admin/features    — create a toggle via Unleash format
//! - GET  /api/admin/features/{name} — get single toggle
//!
//! ## Response format (Unleash v2 wire protocol)
//! {
//!   "version": 2,
//!   "features": [
//!     {
//!       "name": "my-toggle",
//!       "description": "...",
//!       "enabled": true,
//!       "strategies": [{"name": "default", "parameters": {}}],
//!       "variants": [],
//!       "createdAt": "2024-01-01T00:00:00Z",
//!       "lastSeenAt": null,
//!       "impressionData": false
//!     }
//!   ]
//! }

use crate::{models::*, FlagsState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Unleash wire types
// ---------------------------------------------------------------------------

/// Unleash client/admin features response — top-level envelope.
#[derive(Debug, Serialize)]
pub struct UnleashFeaturesResponse {
    pub version: u8,
    pub features: Vec<UnleashToggle>,
}

/// A single Unleash toggle in the response.
#[derive(Debug, Serialize)]
pub struct UnleashToggle {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    /// Activation strategies
    pub strategies: Vec<UnleashStrategy>,
    /// A/B variants (empty until variant support lands)
    pub variants: Vec<serde_json::Value>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "lastSeenAt")]
    pub last_seen_at: Option<DateTime<Utc>>,
    #[serde(rename = "impressionData")]
    pub impression_data: bool,
}

/// An Unleash activation strategy entry.
#[derive(Debug, Serialize)]
pub struct UnleashStrategy {
    pub name: String,
    pub parameters: serde_json::Value,
    /// Segment-based constraints (empty for now)
    #[serde(default)]
    pub constraints: Vec<serde_json::Value>,
}

/// Request body for POST /api/admin/features (Unleash admin create).
#[derive(Debug, Deserialize)]
pub struct UnleashCreateToggleRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(rename = "impressionData", default)]
    pub impression_data: bool,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Strategy conversion: cave-native → Unleash wire format
// ---------------------------------------------------------------------------

fn strategy_to_unleash(strategy: &Strategy) -> UnleashStrategy {
    match strategy {
        Strategy::Default { enabled } => UnleashStrategy {
            name: "default".to_string(),
            parameters: serde_json::json!({ "enabled": enabled }),
            constraints: vec![],
        },
        Strategy::GradualRollout { percentage, group_id } => UnleashStrategy {
            name: "gradualRolloutRandom".to_string(),
            parameters: serde_json::json!({
                "percentage": percentage.to_string(),
                "groupId": group_id.as_deref().unwrap_or("default")
            }),
            constraints: vec![],
        },
        Strategy::UserIds { user_ids } => UnleashStrategy {
            name: "userWithId".to_string(),
            parameters: serde_json::json!({
                "userIds": user_ids.join(",")
            }),
            constraints: vec![],
        },
        Strategy::TenantScope { tenant_ids } => UnleashStrategy {
            name: "flexibleRollout".to_string(),
            parameters: serde_json::json!({
                "rollout": "100",
                "stickiness": "tenantId",
                "groupId": tenant_ids.join(",")
            }),
            constraints: vec![],
        },
        Strategy::EnvironmentScope { environments } => UnleashStrategy {
            name: "environmentScope".to_string(),
            parameters: serde_json::json!({
                "environments": environments.join(",")
            }),
            constraints: vec![],
        },
        Strategy::Custom { name, parameters } => UnleashStrategy {
            name: name.clone(),
            parameters: parameters.clone(),
            constraints: vec![],
        },
    }
}

fn flag_to_unleash(flag: &FeatureFlag) -> UnleashToggle {
    UnleashToggle {
        name: flag.name.clone(),
        description: flag.description.clone(),
        enabled: flag.enabled && !flag.kill_switch,
        strategies: vec![strategy_to_unleash(&flag.strategy)],
        variants: vec![],
        created_at: flag.created_at,
        last_seen_at: None,
        impression_data: false,
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn unleash_router(state: Arc<FlagsState>) -> Router {
    Router::new()
        // Client SDK polling (read-only, no auth required in Unleash OSS)
        .route("/api/client/features", get(client_features))
        // Admin API (create, list, get)
        .route("/api/admin/features", get(admin_list_features).post(admin_create_feature))
        .route("/api/admin/features/{name}", get(admin_get_feature))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// GET /api/client/features — Unleash client SDK polling
// ---------------------------------------------------------------------------

async fn client_features(
    State(_state): State<Arc<FlagsState>>,
) -> Json<UnleashFeaturesResponse> {
    // TODO: load flags from DB via FlagStore
    let flags: Vec<FeatureFlag> = vec![];
    let features = flags.iter().map(flag_to_unleash).collect();
    Json(UnleashFeaturesResponse { version: 2, features })
}

// ---------------------------------------------------------------------------
// GET /api/admin/features — Unleash admin list
// ---------------------------------------------------------------------------

async fn admin_list_features(
    State(_state): State<Arc<FlagsState>>,
) -> Json<UnleashFeaturesResponse> {
    // TODO: load flags from DB — admin sees all including disabled
    let flags: Vec<FeatureFlag> = vec![];
    let features = flags.iter().map(flag_to_unleash).collect();
    Json(UnleashFeaturesResponse { version: 2, features })
}

// ---------------------------------------------------------------------------
// POST /api/admin/features — Unleash admin create toggle
// ---------------------------------------------------------------------------

async fn admin_create_feature(
    State(_state): State<Arc<FlagsState>>,
    Json(req): Json<UnleashCreateToggleRequest>,
) -> (StatusCode, Json<UnleashToggle>) {
    let now = Utc::now();
    let flag = FeatureFlag {
        id: Uuid::new_v4(),
        name: req.name.clone(),
        description: req.description.clone(),
        enabled: req.enabled,
        flag_type: FlagType::Boolean,
        strategy: Strategy::Default { enabled: req.enabled },
        environments: vec![],
        tenant_id: None,
        kill_switch: false,
        created_at: now,
        updated_at: now,
        created_by: Uuid::new_v4(), // TODO: extract from auth context
    };
    // TODO: persist to DB
    (StatusCode::CREATED, Json(flag_to_unleash(&flag)))
}

// ---------------------------------------------------------------------------
// GET /api/admin/features/{name} — Unleash admin get single toggle
// ---------------------------------------------------------------------------

async fn admin_get_feature(
    State(_state): State<Arc<FlagsState>>,
    Path(name): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    // TODO: load from DB by name
    tracing::debug!(flag = %name, "unleash admin_get_feature");
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "name": name,
            "id": "feature-not-found",
            "message": "Feature toggle not found"
        })),
    )
}
