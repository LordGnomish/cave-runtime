// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Synthetic monitoring — compatible with Uptime Kuma
//!
//! Compatible with: Uptime Kuma
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;

pub mod engine;
pub mod history;
pub mod models;
pub mod probe;
pub mod retry;
pub mod routes;
pub mod scheduler;
pub mod status;
pub mod store;

use axum::Router;
use history::HeartbeatStore;
use scheduler::{ProbeScheduler, SchedulerConfig};
use store::ProbeStore;

/// Module-wide application state shared across all request handlers.
pub struct AppState {
    pub probes: ProbeStore,
    pub heartbeats: HeartbeatStore,
    pub scheduler: ProbeScheduler,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            probes: ProbeStore::new(),
            heartbeats: HeartbeatStore::new(500),
            scheduler: ProbeScheduler::new(SchedulerConfig::default()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Kept for backwards compatibility (the old `State` type was empty).
#[derive(Default)]
pub struct State {}

/// Create the axum router for this module.
pub fn router(state: Arc<AppState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "uptime";
