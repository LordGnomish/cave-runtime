// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Secrets — Secret detection engine.
//!
//! Compatible with: Trufflehog + gitleaks
//! Pre-commit and CI secret scanning. Regex + entropy-based detection.

pub mod archive;
pub mod baseline;
pub mod custom_rules;
pub mod decoders;
pub mod detector;
pub mod models;
pub mod precommit;
pub mod routes;
pub mod sanitizer;

use axum::Router;
use std::sync::Arc;

pub struct SecretsState {
    pub detectors: Vec<detector::SecretDetector>,
}

impl Default for SecretsState {
    fn default() -> Self {
        Self {
            detectors: detector::builtin_detectors(),
        }
    }
}

pub fn router(state: Arc<SecretsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "secrets";
