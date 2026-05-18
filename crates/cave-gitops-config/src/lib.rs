// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE GitOps Config — Platform-as-a-product promise management.
//!
//! Compatible with: Kratix
//! Promise-based platform API with pipeline transformation and GitOps state management.

pub mod engine;
pub mod models;
pub mod routes;
pub mod store;

use axum::Router;
use routes::GitOpsAppState;
use std::sync::Arc;

pub struct GitOpsConfigState {
    pub app: Arc<GitOpsAppState>,
}

impl Default for GitOpsConfigState {
    fn default() -> Self {
        Self {
            app: Arc::new(GitOpsAppState::default()),
        }
    }
}

pub fn router(state: Arc<GitOpsAppState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "gitops-config";
