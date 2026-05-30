// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Incident management — compatible with Grafana OnCall (incident tracking side)
//!
//! Covers: incident CRUD, lifecycle (open→ack→resolve→close), timeline/audit log,
//! responders, postmortems, incident metrics (MTTA/MTTR), on-call schedule
//! query (read-only integration — write path lives in cave-oncall).
//!
//! Upstream: grafana/oncall v1.10.0

pub mod engine;
pub mod grouping;
pub mod models;
pub mod oncall;
pub mod routes;
pub mod store;

use axum::Router;
use std::sync::Arc;

/// Module-level shared state.
pub struct State {
    pub store: Arc<store::IncidentStore>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            store: Arc::new(store::IncidentStore::new()),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "incidents";
