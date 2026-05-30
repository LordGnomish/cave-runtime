// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//! SBOM & dependency intelligence — Dependency-Track parity port.
//!
//! Compatible with: DependencyTrack 4.11.6 (Apache-2.0 upstream).
//! Module tree mirrors `src/main/java/org/dependencytrack/`:
//!
//! * [`sbom::cyclonedx`] — CycloneDX 1.5/1.6 JSON+XML parser
//! * [`sbom::spdx`]      — SPDX 2.3 JSON + tag-value parser
//! * [`components`]      — Component / Project / version graph
//! * [`vuln_intel::nvd`] — NVD CVE 2.0 JSON parser
//! * [`vuln_intel::osv`] — OSV.dev advisory parser
//! * [`vuln_intel::ghsa`]— GitHub Security Advisory GraphQL parser
//! * [`vuln_intel::epss`]— Exploit Prediction Scoring System join
//! * [`vuln_intel::snyk`]— Snyk advisory parser (license-permitting subset)
//! * [`policy`]          — license / vulnerability / age / coordinates evaluators
//! * [`portfolio`]       — per-project risk score + time-series snapshots
//! * [`notifications`]   — Webhook / Slack / Teams / Email / Jira sinks
//! * [`routes`]          — axum REST API v1 surface

use std::sync::Arc;

pub mod audit;
pub mod components;
pub mod engine;
pub mod models;
pub mod notifications;
pub mod policy;
pub mod portfolio;
pub mod routes;
pub mod sbom;
pub mod search;
pub mod vuln_intel;

use axum::Router;
use std::sync::RwLock;

/// Module state — in-memory project / component / vulnerability stores.
#[derive(Default)]
pub struct State {
    pub projects: RwLock<Vec<components::Project>>,
    pub components: RwLock<Vec<components::ComponentRecord>>,
    pub vulnerabilities: RwLock<Vec<models::VulnIntel>>,
    pub policies: RwLock<Vec<policy::Policy>>,
    pub notification_rules: RwLock<Vec<notifications::NotificationRule>>,
    pub snapshots: RwLock<Vec<portfolio::PortfolioSnapshot>>,
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "sbom";

/// Upstream commit pinned for parity port.
pub const UPSTREAM_SHA: &str = "128fd0fa01bed9fcb57abffa3b30047c45941415";
pub const UPSTREAM_VERSION: &str = "v4.11.6";
