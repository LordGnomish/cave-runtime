//! CAVE Rollouts — Progressive delivery engine.
//!
//! Replaces: Flagger / Argo Rollouts
//! Strategies: canary, blue-green, A/B testing, header-based routing.
//! Features: automated metric analysis (success rate, latency p99, error rate),
//! promotion/rollback thresholds, cave-mesh traffic splitting, reusable
//! AnalysisTemplates, rollout steps (setWeight, pause, setHeaderRoute,
//! setMirrorRoute), webhook/Slack notifications, experiment support.

pub mod analysis;
pub mod engine;
pub mod notifications;
pub mod routes;
pub mod store;
pub mod types;

use axum::Router;
use std::sync::Arc;

pub struct RolloutsState {
    pub store: Arc<store::RolloutsStore>,
}

impl RolloutsState {
    pub fn new() -> Self {
        Self {
            store: Arc::new(store::RolloutsStore::new()),
        }
    }
}

impl Default for RolloutsState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(state: Arc<RolloutsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "rollouts";
