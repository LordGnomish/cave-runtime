// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Code quality & SAST — compatible with SonarQube
//!
//! Compatible with: SonarQube
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod ast_rules;
pub mod coverage;
pub mod cpd;
pub mod engine;
pub mod models;
pub mod quality_gates;
pub mod routes;
pub mod rules;
pub mod security_hotspots;
pub mod semgrep;

use axum::Router;

/// Module state.
#[derive(Default)]
pub struct State {}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "scan";

pub mod iac;
pub mod license;
pub mod secrets;
// === S2: Trivy scan engine ===
pub mod analyzer;
pub mod oci;
pub mod report_agg;
pub mod scanners;
pub mod target;
// === S2 end ===
// === S4: Report formats ===
pub mod report;
// === S4 end ===
