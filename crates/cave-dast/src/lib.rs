// SPDX-License-Identifier: AGPL-3.0-or-later
//! Dynamic security testing — compatible with ZAP
//!
//! Compatible with: OWASP ZAP
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod routes;
pub mod models;
pub mod engine;

use axum::Router;

/// Module state.
#[derive(Default)]
pub struct State {}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "dast";
