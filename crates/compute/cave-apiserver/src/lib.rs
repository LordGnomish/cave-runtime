// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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

pub mod admission;
pub mod admission_initializer;
pub mod aggregated_apiserver;
pub mod aggregator_v2;
pub mod audit;
pub mod audit_backends;
pub mod audit_policy_v2;
pub mod audit_worm;
pub mod auth_review;
pub mod beta_apis;
pub mod builtin_admission;
pub mod cel_eval;
pub mod conversion;
pub mod conversion_v1;
pub mod crd_controller;
pub mod discovery;
pub mod discovery_v2;
pub mod dra;
pub mod encryption_provider;
pub mod endpointslice_mirror;
pub mod error;
pub mod etcd_backend;
pub mod field_rbac;
pub mod kep_v1_34;
pub mod map_v2;
pub mod mutating_admission_policy;
pub mod node_restriction;
pub mod openapi_v3;
pub mod pod_security;
pub mod priority_fairness;
pub mod rbac;
pub mod resources;
pub mod routes;
pub mod selectors;
pub mod server_side_apply;
pub mod service_account_token;
pub mod storage_migration;
pub mod storage_registry;
pub mod storage_version;
pub mod store;
pub mod validating_admission_policy;
pub mod vap_advanced;
pub mod watch_cache;
pub mod webhook_admission;

#[cfg(test)]
mod parity_tests;

use std::sync::Arc;
use store::ResourceStore;

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
