//! CAVE Artifacts — Pulp v3-compatible artifact repository engine.
//!
//! Replaces: Pulp
//! Implements: Repository CRUD, versions, content types (RPM/Debian/PyPI/OCI/
//! File/Ansible/Maven), remotes, distributions, publications, content guards,
//! chunked upload, async task queue, import/export, signing, RBAC, repair.

pub mod content;
pub mod distribution;
pub mod models;
pub mod rbac;
pub mod repair;
pub mod repository;
pub mod routes;
pub mod signing;
pub mod tasks;
pub mod upload;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;
use tokio::sync::RwLock;

pub use models::*;

pub struct ArtifactsState {
    pub pool: Arc<CavePool>,
    pub task_queue: Arc<tasks::TaskQueue>,
}

impl ArtifactsState {
    pub fn new(pool: Arc<CavePool>) -> Arc<Self> {
        Arc::new(Self {
            pool,
            task_queue: Arc::new(tasks::TaskQueue::new()),
        })
    }
}

impl Default for ArtifactsState {
    fn default() -> Self {
        todo!("ArtifactsState requires a database pool — use ArtifactsState::new(pool)")
    }
}

pub fn router(state: Arc<ArtifactsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "artifacts";
