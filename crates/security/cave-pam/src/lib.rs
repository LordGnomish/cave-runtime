// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Privileged access management — compatible with Teleport CE
//!
//! Compatible with: Teleport CE
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod access_request;
pub mod engine;
pub mod models;
pub mod node_inventory;
pub mod rbac;
pub mod routes;
pub mod session_recorder;

use axum::Router;

/// Module state.
#[derive(Default)]
pub struct State {}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "pam";
