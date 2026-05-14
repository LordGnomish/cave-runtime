// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LLM chat interface — compatible with LibreChat
//!
//! Compatible with: LibreChat
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod engine;
pub mod models;
pub mod routes;

use axum::Router;

/// Module state.
#[derive(Default)]
pub struct State {}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "chat";
