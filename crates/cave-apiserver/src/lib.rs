//! cave-apiserver — Kubernetes-compatible API server.
//!
//! Implements core K8s resource CRUD with watch support.
//! Resources: Pod, Deployment, Service, ConfigMap, Secret, Namespace.
//!
//! ## API (K8s compatible paths)
//!
//! ```text
//! /api/v1/namespaces                        — Namespace CRUD
//! /api/v1/namespaces/{ns}/pods              — Pod CRUD
//! /api/v1/namespaces/{ns}/services          — Service CRUD
//! /api/v1/namespaces/{ns}/configmaps        — ConfigMap CRUD
//! /api/v1/namespaces/{ns}/secrets           — Secret CRUD
//! /apis/apps/v1/namespaces/{ns}/deployments — Deployment CRUD
//! ```

pub mod error;
pub mod resources;
pub mod store;
pub mod routes;
pub mod admission;
pub mod watch_cache;
pub mod conversion;
pub mod server_side_apply;
pub mod audit;
pub mod rbac;
pub mod aggregated_apiserver;
pub mod endpointslice_mirror;
pub mod etcd_backend;
pub mod discovery;
pub mod crd_controller;
pub mod storage_version;
pub mod selectors;
pub mod priority_fairness;
pub mod validating_admission_policy;
pub mod vap_advanced;
pub mod mutating_admission_policy;
pub mod webhook_admission;
pub mod builtin_admission;
pub mod audit_policy_v2;
pub mod discovery_v2;
pub mod conversion_v1;
pub mod aggregator_v2;
pub mod beta_apis;
pub mod storage_migration;
pub mod service_account_token;
pub mod encryption_provider;
pub mod audit_worm;
pub mod field_rbac;

use store::ResourceStore;
use std::sync::Arc;

pub fn new_state() -> Arc<ResourceStore> {
    Arc::new(ResourceStore::new())
}

pub fn router(state: Arc<ResourceStore>) -> axum::Router {
    routes::create_router(state)
}

/// Calculate parity against the local source tree at compile-time crate root.
pub fn calculate_parity() -> Result<cave_kernel::parity::ParityReport, String> {
    cave_kernel::parity::calculate_from_str(
        include_str!("../parity.manifest.toml"),
        env!("CARGO_MANIFEST_DIR"),
    )
    .map_err(|e| e.to_string())
}
