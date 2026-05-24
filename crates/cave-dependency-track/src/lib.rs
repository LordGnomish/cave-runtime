// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-dependency-track` — SBOM / SCA platform.
//!
//! Upstream: DependencyTrack/dependency-track v4.14.2 (Apache-2.0).
//! Source commit pin: see `parity.manifest.toml`.
//!
//! Surface mirrors `org.dependencytrack.*`:
//!
//! * [`portfolio`]     — Project portfolio: CRUD + hierarchy + tags
//! * [`sbom`]          — CycloneDX 1.4/1.5/1.6 + SPDX 2.3 ingestion
//! * [`components`]    — Component analysis + identity
//! * [`vuln_intel`]    — NVD / OSV / GHSA / Snyk / OSS Index / VulnDB / EPSS
//! * [`policy`]        — license / vulnerability / age / coordinates engine
//! * [`audit`]         — vulnerability audit state machine
//! * [`vex`]           — CycloneDX VEX export
//! * [`bov`]           — Bill of Vulnerabilities export
//! * [`licenses`]      — SPDX license catalog
//! * [`cpe`] / [`purl`] — coordinate parsers
//! * [`risk`]          — inherited risk score
//! * [`notifications`] — Slack / Teams / Mattermost / Email / Jira / Webhook
//! * [`integrations`]  — Defect Dojo / Fortify SSC / Kenna / ThreadFix
//! * [`routes`]        — axum REST v1 + GraphQL surface
//! * [`graphql`]       — GraphQL portfolio query

use std::sync::Arc;

pub mod audit;
pub mod bov;
pub mod components;
pub mod cpe;
pub mod engine;
pub mod error;
pub mod graphql;
pub mod integrations;
pub mod licenses;
pub mod models;
pub mod notifications;
pub mod policy;
pub mod portfolio;
pub mod purl;
pub mod repositories;
pub mod risk;
pub mod routes;
pub mod sbom;
pub mod swagger;
pub mod vex;
pub mod vuln_intel;

use axum::Router;

pub const MODULE_NAME: &str = "deptrack";
pub const UPSTREAM_NAME: &str = "DependencyTrack";
pub const UPSTREAM_VERSION: &str = "v4.14.2";
pub const UPSTREAM_SHA: &str = "c4a156726472cd529cc9fa8ed12e825cc000327d";

/// Module state — in-memory portfolio + vulnerability + policy store.
pub struct State {
    pub portfolio: portfolio::PortfolioStore,
    pub vulns: vuln_intel::VulnStore,
    pub policy: policy::PolicyStore,
    pub audit: audit::AuditStore,
    pub notifications: notifications::NotificationRuleStore,
    pub repositories: repositories::RepositoryStore,
}

impl Default for State {
    fn default() -> Self {
        Self {
            portfolio: portfolio::PortfolioStore::new(),
            vulns: vuln_intel::VulnStore::new(),
            policy: policy::PolicyStore::new(),
            audit: audit::AuditStore::new(),
            notifications: notifications::NotificationRuleStore::new(),
            repositories: repositories::RepositoryStore::default(),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_is_deptrack() {
        assert_eq!(MODULE_NAME, "deptrack");
    }

    #[test]
    fn upstream_pin_is_v4_14_2() {
        assert_eq!(UPSTREAM_VERSION, "v4.14.2");
        assert_eq!(UPSTREAM_SHA.len(), 40);
        assert!(UPSTREAM_SHA.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn default_state_is_empty() {
        let s = State::default();
        assert_eq!(s.portfolio.count(), 0);
        assert_eq!(s.vulns.count(), 0);
        assert_eq!(s.policy.count(), 0);
        assert_eq!(s.audit.count(), 0);
    }

    #[test]
    fn router_builds_from_default_state() {
        let _r = router(Arc::new(State::default()));
    }
}
