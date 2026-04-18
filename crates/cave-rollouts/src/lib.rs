//! CAVE Rollouts — Progressive delivery engine.
//!
//! Replaces: Flagger + Argo Rollouts
//! Upstream tracking: see cave-upstream.
//!
//! ## Supported strategies
//! - Canary (weight-based step analysis, traffic mirroring)
//! - Blue/Green (preview service, auto/manual promotion)
//! - A/B Testing (header-based routing)
//!
//! ## Analysis
//! - AnalysisTemplates (Prometheus, webhook, Datadog, CloudWatch, NewRelic, Job)
//! - AnalysisRuns with metric evaluation and failure limits
//!
//! ## Manual gates
//! - promote / promoteFull / abort / pause / resume / retry via REST API
//!
//! ## Notifications
//! - Slack webhook, generic webhook, Teams, PagerDuty

pub mod engine;
pub mod models;
pub mod routes;
pub mod store;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

/// Module state.
pub struct RolloutsState {
    pub pool: Arc<CavePool>,
}

impl Default for RolloutsState {
    fn default() -> Self {
        Self {
            pool: Arc::new(cave_db::CavePool::mock()),
            
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<RolloutsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "rollouts";
