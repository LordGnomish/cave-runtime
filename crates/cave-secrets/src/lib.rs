//! CAVE Secrets — Secret detection engine.
//!
//! Replaces: Trufflehog + gitleaks
//! Pre-commit and CI secret scanning. Regex + entropy-based detection.

pub mod detector;
pub mod routes;

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
