// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Chaos engineering — compatible with Chaos Mesh
//!
//! Compatible with: Chaos Mesh
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod daemon;
pub mod engine;
pub mod executor;
pub mod models;
pub mod reconcile;
pub mod routes;
pub mod selector;
pub mod store;
pub mod workflow;
pub mod schedule;

use axum::Router;

/// Module state — holds shared store + executor for all HTTP handlers.
pub struct State {
    pub store: Arc<store::ChaosStore>,
    pub executor: Arc<executor::ChaosExecutor>,
}

impl Default for State {
    fn default() -> Self {
        State {
            store: Arc::new(store::ChaosStore::new()),
            executor: Arc::new(executor::ChaosExecutor::new()),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "chaos";
