//! CAVE Runbook — Automated runbook execution engine.
//!
//! Replaces: Rundeck, StackStorm
//! Sequential and parallel step execution with approval gates, scheduling, and full output capture.

pub mod engine;
pub mod models;
pub mod routes;
pub mod store;

use axum::Router;
use routes::RunbookAppState;
use std::sync::Arc;

pub struct RunbookState {
    pub app: Arc<RunbookAppState>,
}

impl Default for RunbookState {
    fn default() -> Self {
        Self {
            app: Arc::new(RunbookAppState::default()),
        }
    }
}

pub fn router(state: Arc<RunbookAppState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "runbook";
