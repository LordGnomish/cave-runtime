// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Deploy — GitOps deployment engine (ArgoCD parity).
//!
//! Modules:
//! - [`models`]        — Application / AppProject / AppSet / sync / health CRDs
//! - [`appset`]        — generators + ApplicationSet reconciliation primitives
//! - [`gitops`]        — sync + drift + manifest render
//! - [`sync`]          — sync waves, hooks, rollback metadata, sync options
//! - [`rollout`]       — canary / blue-green / rolling progressive delivery
//! - [`diff`]          — structured diff between desired and live JSON
//! - [`health`]        — Kubernetes resource health assessors
//! - [`rbac`]          — AppProject scope + role policy evaluation
//! - [`cluster`]       — cluster registry + Kubernetes URL builders
//! - [`notifications`] — Slack / webhook engine on top of [`models::NotificationConfig`]
//! - [`store`]         — in-memory CRUD store (Phase 2 will land cave-db wiring)
//! - [`error`]         — DeployError + IntoResponse for the axum API
//! - [`routes`]        — HTTP API

pub mod appset;
pub mod cluster;
pub mod diff;
pub mod error;
pub mod gitops;
pub mod health;
pub mod helm_deps;
pub mod image_updater;
pub mod models;
pub mod notifications;
pub mod rbac;
pub mod rollout;
pub mod routes;
pub mod store;
pub mod sync;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

pub use error::DeployError;
pub use models::*;
pub use store::DeployStore;

pub struct DeployState {
    pub pool: Arc<CavePool>,
    pub store: Arc<DeployStore>,
}

impl DeployState {
    pub fn new(pool: Arc<CavePool>) -> Arc<Self> {
        Arc::new(Self {
            pool,
            store: Arc::new(DeployStore::new()),
        })
    }
}

impl Default for DeployState {
    fn default() -> Self {
        Self {
            pool: Arc::new(cave_db::CavePool::mock()),
            store: Arc::new(DeployStore::new()),
        }
    }
}

pub fn router(state: Arc<DeployState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "deploy";
