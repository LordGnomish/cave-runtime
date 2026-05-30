// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LLM chat interface — compatible with LibreChat
//!
//! Compatible with: LibreChat v0.7.6 (danny-avila/LibreChat)
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod conversation_tree;
pub mod engine;
pub mod models;
pub mod routes;
pub mod store;

pub use routes::AppState;

use axum::Router;

/// Create the axum router for this module backed by a shared AppState.
pub fn router(state: Arc<AppState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "chat";
