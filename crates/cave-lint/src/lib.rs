//! CAVE Lint — Config & image linting engine.
//!
//! Replaces: Hadolint + Checkov + Pluto + kubent
//! Dockerfile linting, K8s manifest validation, deprecated API detection.

pub mod rules;
pub mod routes;

use axum::Router;
use std::sync::Arc;

pub struct LintState {
    pub rules: Vec<rules::LintRule>,
}

impl Default for LintState {
    fn default() -> Self {
        Self {
            rules: rules::builtin_rules(),
        }
    }
}

pub fn router(state: Arc<LintState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "lint";
