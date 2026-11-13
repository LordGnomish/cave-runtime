// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DevOps analytics — compatible with DevLake
//!
//! Compatible with: DevLake
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod engine;
pub mod models;
pub mod routes;
pub mod store;

use axum::Router;

/// Module state — holds the in-memory DevLake store.
pub struct State {
    pub store: store::DevlakeStore,
}

impl Default for State {
    fn default() -> Self {
        Self {
            store: store::DevlakeStore::new(),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "devlake";
