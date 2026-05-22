// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Developer portal — compatible with Backstage.

pub mod catalog;
pub mod dashboard;
pub mod engine;
pub mod models;
pub mod plugins;
pub mod routes;
pub mod ui;

/// Per-module admin views (etcd / cri / apiserver / iam / mesh / pg / vault)
/// + per-tenant dashboard. Pinned to backstage/backstage v1.50.3.
pub mod admin;

/// Live-runtime data sources for Portal admin pages that read from
/// non-apiserver backends (currently: `cave-auth` admin REST).
/// Companion to `admin::runtime_client`.
pub mod runtime_client;

use axum::Router;
use cave_kernel::parity::ParityReport;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PortalState {
    pub services: RwLock<Vec<models::Service>>,
    /// Parity reports collected from each module, keyed by module name.
    pub parity_cache: RwLock<HashMap<String, ParityReport>>,
}

impl Default for PortalState {
    fn default() -> Self {
        Self {
            services: RwLock::new(Vec::new()),
            parity_cache: RwLock::new(HashMap::new()),
        }
    }
}

pub type State = PortalState;

pub fn router(state: Arc<PortalState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "portal";
