// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SLO tracking engine — error budgets, burn rates
//!
//! Compatible with: Custom (Prometheus rules)
//! Upstream tracking: nobl9/nobl9-go v0.126.1

use std::sync::Arc;
pub mod engine;
pub mod models;
pub mod routes;
pub mod store;

use axum::Router;

/// Module state — holds the in-memory SLO store.
#[derive(Default)]
pub struct State {
    pub store: Arc<store::SloStore>,
}

impl State {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            store: store::SloStore::new(),
        })
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "slo";
