//! CAVE Pipelines — CI/CD pipeline engine.
//!
//! Replaces: Tekton Pipelines + Jenkins
//! Implements: Pipeline/Task CRDs, DAG execution, triggers, catalog,
//! Jenkins Jenkinsfile compatibility, artifact passing, log streaming.

pub mod catalog;
pub mod engine;
pub mod jenkins;
pub mod models;
pub mod routes;
pub mod triggers;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;
use tokio::sync::RwLock;

pub use models::*;

/// Module-level state shared across all handlers.
pub struct PipelinesState {
    pub pool: Arc<CavePool>,
    /// In-memory pipeline run tracking (augments DB for live log streaming).
    pub active_runs: Arc<RwLock<std::collections::HashMap<uuid::Uuid, engine::RunHandle>>>,
    /// Built-in task catalog.
    pub catalog: Arc<catalog::TaskCatalog>,
}

impl PipelinesState {
    pub fn new(pool: Arc<CavePool>) -> Arc<Self> {
        Arc::new(Self {
            pool,
            active_runs: Arc::new(RwLock::new(std::collections::HashMap::new())),
            catalog: Arc::new(catalog::TaskCatalog::builtin()),
        })
    }
}

impl Default for PipelinesState {
    fn default() -> Self {
        Self {
            pool: Arc::new(cave_db::CavePool::mock()),
            active_runs: Arc::new(RwLock::new(std::collections::HashMap::new())),
            catalog: Arc::new(catalog::TaskCatalog::builtin()),
        }
    }
}

/// Legacy alias used by cave-runtime.
pub type State = PipelinesState;

pub fn router(state: Arc<PipelinesState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "pipelines";
