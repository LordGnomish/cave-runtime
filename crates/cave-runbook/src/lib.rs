// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE Runbook — Runbook automation engine.
//! Compatible with: manual runbooks, PagerDuty runbooks.
//! Features: YAML-defined runbooks, multi-step execution, approvals, scheduling.

pub mod engine;
pub mod library;
pub mod models;
pub mod routes;
pub mod schedule;
pub mod steps;

use axum::Router;
use models::*;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub struct RunbookStore {
    pub runbooks: HashMap<uuid::Uuid, Runbook>,
    pub executions: HashMap<uuid::Uuid, Execution>,
    pub approval_requests: HashMap<uuid::Uuid, ApprovalRequest>,
    pub triggers: HashMap<uuid::Uuid, RunbookTrigger>,
}

impl Default for RunbookStore {
    fn default() -> Self {
        let mut store = RunbookStore {
            runbooks: HashMap::new(),
            executions: HashMap::new(),
            approval_requests: HashMap::new(),
            triggers: HashMap::new(),
        };
        // Seed built-in templates
        for template in library::builtin_templates() {
            store.runbooks.insert(template.id, template);
        }
        store
    }
}

pub struct RunbookState {
    pub store: Arc<RwLock<RunbookStore>>,
}

impl Default for RunbookState {
    fn default() -> Self {
        Self {
            store: Arc::new(RwLock::new(RunbookStore::default())),
        }
    }
}

pub fn router(state: Arc<RunbookState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "runbook";
