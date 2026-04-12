//! Automated runbook execution and incident remediation.
//!
//! Replaces: PagerDuty Runbooks / Rundeck / StackStorm
//!
//! Core concepts:
//! - Runbook: ordered set of steps triggered manually, by incidents, alerts, or schedule.
//! - RunbookExecution: a live or completed run with per-step results.
//! - IncidentBinding: auto-attach a runbook when an incident matches a pattern.
//! - ApprovalRequest: human-in-the-loop gate; execution pauses until approved/rejected.
//! - RunbookTemplate: predefined runbook skeletons (restart, scale, failover, etc.).

pub mod executor;
pub mod models;
pub mod routes;
pub mod templates;
pub mod triggers;

use axum::Router;
use models::{ApprovalRequest, IncidentBinding, Runbook, RunbookExecution};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Shared module state — all fields behind `Arc<Mutex<_>>` for concurrent access.
pub struct RunbookState {
    pub runbooks: Mutex<HashMap<Uuid, Runbook>>,
    pub executions: Mutex<HashMap<Uuid, RunbookExecution>>,
    pub bindings: Mutex<HashMap<Uuid, IncidentBinding>>,
    pub approvals: Mutex<HashMap<Uuid, ApprovalRequest>>,
}

impl Default for RunbookState {
    fn default() -> Self {
        Self {
            runbooks: Mutex::new(HashMap::new()),
            executions: Mutex::new(HashMap::new()),
            bindings: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<RunbookState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "runbook";
