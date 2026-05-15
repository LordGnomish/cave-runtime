// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pulp v3-compatible artifact repository module.
//!
//! Compatible with: Pulp v3
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

pub use models::*;

/// Pulp module state. Kept as `ArtifactsState` for source-level back-compat
/// with handlers in `routes.rs`; aliased as `PulpState` for new callers that
/// want a name-distinguished variant of the multi-upstream state graph.
pub struct ArtifactsState {
    pub pool: Arc<CavePool>,
    pub task_queue: Arc<tasks::TaskQueue>,
}

pub type PulpState = ArtifactsState;

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
        Self {
            pool: Arc::new(cave_db::CavePool::mock()),
            task_queue: Arc::new(tasks::TaskQueue::new()),
        }
    }
}

pub fn router(state: Arc<ArtifactsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "pulp";
