//! CAVE Tracker — Issue tracking and project management.
//!
//! Replaces: Jira
//! Full issue lifecycle management with sprints, epics, JQL queries, and workflow engine.

pub mod jql;
pub mod models;
pub mod routes;
pub mod store;
pub mod workflow;

use axum::Router;
use std::sync::Arc;

pub struct TrackerState {
    pub store: Arc<store::TrackerStore>,
    pub workflow: Arc<workflow::WorkflowEngine>,
}

impl Default for TrackerState {
    fn default() -> Self {
        Self {
            store: Arc::new(store::TrackerStore::new()),
            workflow: Arc::new(workflow::WorkflowEngine::new()),
        }
    }
}

pub fn router(state: Arc<TrackerState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "tracker";
