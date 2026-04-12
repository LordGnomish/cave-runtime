//! CAVE Registry — Docker/OCI/Harbor-compatible container registry.
//!
//! Replaces: Harbor
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
}

impl Default for RegistryState {
    fn default() -> Self {
        todo!("RegistryState requires a database pool and storage backend")
    }
}

/// Build the combined axum router (Docker V2 + Harbor Admin API).
pub fn router(state: Arc<RegistryState>) -> Router {
    routes::v2::router(Arc::clone(&state))
        .merge(routes::harbor::router(state))
}

pub const MODULE_NAME: &str = "registry";
