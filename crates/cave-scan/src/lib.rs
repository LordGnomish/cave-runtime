// SPDX-License-Identifier: AGPL-3.0-or-later
//! Code quality & SAST — compatible with SonarQube
//!
//! Compatible with: SonarQube
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
// NOTE: coverage module is safe, but rules.rs has pre-existing Unicode/raw-string
// syntax errors preventing compilation (em-dashes, raw strings with quote issues).
// Coverage is usable; rules require syntax repair in a separate effort.
pub mod coverage;
pub mod engine;
pub mod models;
pub mod routes;
// pub mod rules;  // TODO: Fix raw string escaping in rules.rs

use axum::Router;

/// Module state.
#[derive(Default)]
pub struct State {}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "scan";
