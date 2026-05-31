// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: META — cave-artifacts harbor sub-module root
//! Harbor / Docker / OCI container registry module.
//!
//! Compatible with: Harbor v2.10
//! Upstream tracking: see cave-upstream.
//!
//! ## Implemented
//! - Docker Registry V2 API (all 16 endpoints)
//! - OCI Distribution Spec 1.1 (referrers, artifacts)
//! - OCI Image Spec (manifest, index, config, layers)
//! - Content-addressable blob storage (SHA-256)
//! - Garbage collection
//! - Harbor Admin API v2.0: projects, robot accounts, vulnerability scanning,
//!   replication rules, tag retention, immutable tags, webhooks, quotas,
//!   audit logs, labels, P2P preheat, LDAP/OIDC config

pub mod gc;
pub mod harbor;
pub mod models;
pub mod pipeline;
pub mod project_store;
pub mod proxy;
pub mod quota;
pub mod rbac;
pub mod replication_reconciler;
pub mod retention;
pub mod routes;
pub mod storage;
pub mod store;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;
use storage::RegistryStorage;

/// Module state shared across all handlers.
pub struct RegistryState {
    pub pool: Arc<CavePool>,
    pub storage: Arc<RegistryStorage>,
    pub projects: Arc<project_store::ProjectStore>,
    pub proxy: proxy::ProxyClient,
    pub pipeline: pipeline::ScanPipeline,
}

impl Default for RegistryState {
    fn default() -> Self {
        Self {
            pool: Arc::new(cave_db::CavePool::mock()),
            storage: Arc::new(RegistryStorage::default()),
            projects: Arc::new(project_store::ProjectStore::new()),
            proxy: proxy::ProxyClient::new(proxy::ProxyConfig::default()),
            pipeline: pipeline::ScanPipeline::new(pipeline::ScanPipelineConfig::default()),
        }
    }
}

/// Build the combined axum router (Docker V2 + Harbor Admin API + proxy).
pub fn router(state: Arc<RegistryState>) -> Router {
    routes::v2::router(Arc::clone(&state))
        .merge(routes::harbor::router(Arc::clone(&state)))
        .merge(routes::proxy::router(state))
}

pub const MODULE_NAME: &str = "harbor";
