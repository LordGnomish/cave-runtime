// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE Flags — Unleash-compatible feature flag service.
//!
//! Compatible with: Unleash
//! Upstream tracking: see cave-upstream (Unleash v6.x).
//!
//! ## Implemented
//! - Feature toggle types: release, experiment, operational, kill-switch, permission
//! - Activation strategies: default, userWithId, gradualRolloutRandom,
//!   gradualRolloutSessionId, gradualRolloutUserId, flexibleRollout,
//!   remoteAddress, applicationHostname, custom
//! - Constraints (all operators): IN/NOT_IN, STR_*, NUM_*, DATE_*, SEMVER_*
//! - Segments (reusable constraint groups)
//! - Variants (weighted, stickiness, payload: string/json/csv)
//! - Feature environments (enable/disable per env)
//! - Projects with default strategies
//! - Impression data events
//! - Metrics collection (/api/client/metrics)
//! - SDK compatibility: client (server-side) + frontend APIs
//! - Unleash Admin API (full CRUD)
//! - Stale feature detection
//! - Change requests
//! - Banners

pub mod engine;
pub mod models;
pub mod routes;
pub mod store;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;
use tokio::sync::RwLock;

use models::{FeatureFlag, Segment};

/// In-memory cache for hot-path evaluation (refreshed from DB on write).
#[derive(Default)]
pub struct FeatureCache {
    pub features: Vec<FeatureFlag>,
    pub segments: Vec<Segment>,
}

/// Module state shared across request handlers.
pub struct FlagsState {
    pub pool: Arc<CavePool>,
    /// Read-through cache so the high-frequency client SDK polling
    /// does not hit Postgres on every request.
    pub cache: Arc<RwLock<FeatureCache>>,
}

impl FlagsState {
    pub fn new(pool: Arc<CavePool>) -> Self {
        Self {
            pool,
            cache: Arc::new(RwLock::new(FeatureCache::default())),
        }
    }
}

impl Default for FlagsState {
    fn default() -> Self {
        Self {
            pool: Arc::new(cave_db::CavePool::mock()),
            cache: Arc::new(RwLock::new(FeatureCache::default())),
        }
    }
}

/// Create the axum router for the flags module.
pub fn router(state: Arc<FlagsState>) -> Router {
    routes::create_router(state)
}

/// Module name for DB schema.
pub const MODULE_NAME: &str = "flags";
