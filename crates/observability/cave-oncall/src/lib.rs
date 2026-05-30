// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! On-call scheduling, paging, and escalation — compatible with Grafana OnCall.
//!
//! Compatible with: Grafana OnCall
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod engine;
pub mod integrations;
pub mod models;
pub mod pagerduty_migrator;
pub mod routes;
pub mod slack;
pub mod sms_voice;

pub use engine::OnCallError;
pub use routes::OnCallStore;

use axum::Router;

/// Create the axum router for this module.
pub fn router(state: Arc<OnCallStore>) -> Router {
    routes::create_router(state)
}

/// Convenience: build a fresh `OnCallStore` wrapped in an `Arc`.
pub fn new_state() -> Arc<OnCallStore> {
    Arc::new(OnCallStore::default())
}

pub const MODULE_NAME: &str = "oncall";

pub type State = OnCallStore;
