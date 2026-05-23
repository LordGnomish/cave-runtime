// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-trufflehog` — TruffleHog-parity secret scanner.
//!
//! Upstream: trufflesecurity/trufflehog v3.95.3 (AGPL-3.0-only). Source
//! commit pin: see `parity.manifest.toml`. cave-trufflehog is
//! AGPL-3.0-or-later; AGPL upstream is fully compatible.
//!
//! Surface:
//!   * 18 first-party high-signal detectors (AWS, GitHub, GitLab, Slack,
//!     Stripe, Anthropic, OpenAI, Twilio, SendGrid, GCP, Azure, Mailgun,
//!     Square, npm, PyPI, JWT, private key, generic high-entropy)
//!   * Custom detector loader (YAML / TOML with regex + entropy + multi-step
//!     HTTP verification + `successRanges` + `rotatedRanges`)
//!   * Source connectors: Git (working tree + history), GitHub, GitLab,
//!     Bitbucket, S3, GCS, Filesystem, Docker, Stdin, JIRA, Confluence,
//!     Slack, Postgres/MySQL/SQLite dump
//!   * Engine: chunker, worker pool, dedup, rate-limit, resume checkpoints
//!   * Decoders: base64, utf16, utf8, escaped-unicode, html
//!   * Live verification with `VerificationCache` + `SuccessRanges` /
//!     `RotatedRanges` per-status semantics
//!   * Output writers: JSON, JSONL, GitHub Actions, plain
//!   * Portal axum API + Prometheus metrics + alerting rules
//!
//! Out of MVP — see `[[scope_cuts]]` in `parity.manifest.toml`.

use std::sync::{Arc, Mutex};

pub mod chunker;
pub mod config;
pub mod custom_detectors;
pub mod decoders;
pub mod dedup;
pub mod detector;
pub mod detectors;
pub mod engine;
pub mod error;
pub mod job_progress;
pub mod metrics;
pub mod models;
pub mod output;
pub mod resume;
pub mod routes;
pub mod sources;
pub mod store;
pub mod verification;

use config::ScanConfig;
use detector::DetectorRegistry;
use store::FindingStore;
use verification::VerificationCache;

pub const MODULE_NAME: &str = "trufflehog";

/// Module state. Bundles the active detector registry, finding store,
/// verification cache, and scan-time config the HTTP routes consult.
pub struct State {
    pub config: ScanConfig,
    pub registry: DetectorRegistry,
    pub store: FindingStore,
    pub verification: Arc<VerificationCache>,
    pub job_progress: Mutex<job_progress::JobProgress>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            config: ScanConfig::default(),
            registry: DetectorRegistry::builtin(),
            store: FindingStore::new(),
            verification: Arc::new(VerificationCache::new(1024)),
            job_progress: Mutex::new(job_progress::JobProgress::new()),
        }
    }
}

impl State {
    pub fn with_config(cfg: ScanConfig) -> Self {
        Self {
            config: cfg,
            ..Default::default()
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> axum::Router {
    routes::create_router(state)
}

/// Return the count of built-in detectors. Used by self-audit + smoke tests.
pub fn builtin_detector_count() -> usize {
    DetectorRegistry::builtin().detectors.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_is_trufflehog() {
        assert_eq!(MODULE_NAME, "trufflehog");
    }

    #[test]
    fn default_state_has_builtin_detectors() {
        let s = State::default();
        assert!(s.registry.detectors.len() >= 16);
    }

    #[test]
    fn builtin_detector_count_matches_state() {
        let s = State::default();
        assert_eq!(builtin_detector_count(), s.registry.detectors.len());
    }
}
