// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LLM observability — compatible with Langfuse
//!
//! Compatible with: Langfuse
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod engine;
pub mod models;
pub mod prompt;
pub mod routes;
pub mod trace_models;
pub mod trace_store;

use axum::Router;

/// Module state.
#[derive(Default)]
pub struct State {
    pub store: trace_store::TraceStore,
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "ai-obs";
