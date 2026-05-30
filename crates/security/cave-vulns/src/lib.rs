// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vulnerability aggregation hub — compatible with DefectDojo
//! Vulnerability aggregation hub — DefectDojo-parity finding pipeline.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 (pinned upstream).
//!
//! ## What's in the box
//!
//! - `finding` — DefectDojo-parity Finding model + state machine
//! - `dedup` — four canonical algorithms (legacy / hash_code /
//!   unique_id / unique_id_or_hash_code) + per-scanner field tuples
//! - `cvss` — v3.1 + v4.0 vector parser + base-score calculators
//! - `hierarchy` — ProductType → Product → Engagement → Test
//! - `parsers` — Bandit / Trivy / ZAP / Semgrep / SARIF / Snyk / Nuclei
//! - `risk_accept` — RiskAcceptance workflow with expiry/reactivation
//! - `sla` — per-severity windows, breach detection, rollup
//! - `reports` — JSON + HTML executive summary
//! - `notifications` — pluggable sinks (InMemory / Webhook / Log)
//! - `routes` — DefectDojo API v2 surface
//! - `models` / `engine` / `dedup` (legacy modules kept for compat)

use axum::Router;
use cave_db::Storage;
use std::sync::Arc;

// New first-class modules (deep DefectDojo port).
pub mod cvss;
pub mod endpoint;
pub mod engagement_scope;
pub mod finding;
pub mod hierarchy;
pub mod lifecycle;
pub mod notification_rules;
pub mod notifications;
pub mod parsers;
pub mod reports;
pub mod risk_accept;
pub mod sla;

// Legacy modules preserved to keep existing portal/cavectl call-sites
// compiling. They share types with the new modules where it makes sense.
pub mod dedup;
pub mod engine;
pub mod models;
pub mod routes;

/// Module state.
pub struct State {
    pub storage: Arc<dyn Storage>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            storage: Arc::new(cave_db::MemoryStorage::default()),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "vulns";
pub const UPSTREAM_NAME: &str = "DefectDojo";
pub const UPSTREAM_VERSION: &str = "v2.58.2";
pub const UPSTREAM_SHA: &str = "6eab87386d504c4bc164f87b6aae58a8e0c1b8d2";
