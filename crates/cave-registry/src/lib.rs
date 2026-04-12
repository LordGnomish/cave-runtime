//! CAVE Registry — container image registry and package proxy/cache.
//!
//! Replaces Pulp / Harbor with a Rust-native implementation.
//! Implements the Docker Registry HTTP API V2 / OCI Distribution Spec
//! so that `docker push/pull`, Helm, and any OCI-compatible tool works
//! against cave-registry without configuration changes.
//!
//! ## Upstream Compatibility: Docker Registry V2
//! - Version check:    GET  /v2/
//! - Catalog:          GET  /v2/_catalog
//! - Manifests:        GET/HEAD/PUT /v2/:name/manifests/:reference
//! - Blobs:            GET/HEAD     /v2/:name/blobs/:digest
//! - Blob upload:      POST/PATCH/PUT /v2/:name/blobs/uploads/
//! - Tags:             GET  /v2/:name/tags/list
//!
//! ## Upstream Tracking
//! - OCI Distribution Spec: https://github.com/opencontainers/distribution-spec
//! - Docker Registry API:   https://docs.docker.com/registry/spec/api/

pub mod docker_v2;
pub mod routes;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

/// Module state.
pub struct State {
    pub pool: Arc<CavePool>,
}

/// Create the axum router for this module.
///
/// Merges cave-native management routes with the Docker Registry V2
/// compatible API so that OCI clients work without modification.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(Arc::clone(&state))
        .merge(docker_v2::docker_v2_router(state))
}

pub const MODULE_NAME: &str = "registry";
