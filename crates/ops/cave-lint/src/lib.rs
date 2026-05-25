// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Lint — Config & image linting engine.
//!
//! Compatible with: Hadolint + Checkov + Pluto + kubent
//! Dockerfile linting, K8s manifest validation, deprecated API detection.

pub mod routes;
pub mod rules;

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
