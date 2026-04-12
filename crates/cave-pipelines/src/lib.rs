//! CI/CD pipeline engine — replaces Tekton / Jenkins
//!
//! Replaces: Tekton, Jenkins
//! Upstream tracking: see cave-upstream for monitored features.
//!
//! # Architecture
//!
//! - `models`   — core data types (Pipeline, PipelineRun, Task, TaskRun, …)
//! - `engine`   — DAG ordering, conditional execution, parameter interpolation
//! - `executor` — step runner (child process + stdout/stderr capture)
//! - `triggers` — webhook (GitHub/GitLab), cron, manual triggers
//! - `workspace`— shared-volume management between tasks
//! - `catalog`  — built-in and custom task/pipeline templates
//! - `build`    — build strategy config (Dockerfile, Buildpacks, Kaniko, S2I)
//! - `notifications` — webhook, Slack, email on status change
//! - `github`   — commit status and PR check-run integration
//! - `routes`   — Axum HTTP admin API

pub mod build;
pub mod catalog;
pub mod engine;
pub mod executor;
pub mod github;
pub mod models;
pub mod notifications;
pub mod routes;
pub mod triggers;
pub mod workspace;

use axum::Router;
use catalog::TaskCatalog;
use models::{ApprovalGate, Pipeline, PipelineRun, Task, TaskRun};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use triggers::Trigger;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------

/// All mutable pipeline state, protected behind a single async mutex.
pub struct PipelineStore {
    pub pipelines: HashMap<Uuid, Pipeline>,
    pub pipeline_runs: HashMap<Uuid, PipelineRun>,
    pub tasks: HashMap<Uuid, Task>,
    pub task_runs: HashMap<Uuid, TaskRun>,
    pub triggers: HashMap<Uuid, Trigger>,
    pub approvals: HashMap<Uuid, ApprovalGate>,
}

impl PipelineStore {
    pub fn new() -> Self {
        Self {
            pipelines: HashMap::new(),
            pipeline_runs: HashMap::new(),
            tasks: HashMap::new(),
            task_runs: HashMap::new(),
            triggers: HashMap::new(),
            approvals: HashMap::new(),
        }
    }
}

impl Default for PipelineStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Module state
// ---------------------------------------------------------------------------

/// Module state (passed as `Arc<State>` to all handlers).
pub struct State {
    pub store: Mutex<PipelineStore>,
    pub catalog: TaskCatalog,
}

impl State {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(PipelineStore::new()),
            catalog: TaskCatalog::new(),
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create the Axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "pipelines";
