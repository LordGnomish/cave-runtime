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

use store::ResourceStore;
use std::sync::Arc;

pub fn new_state() -> Arc<ResourceStore> {
    Arc::new(ResourceStore::new())
}

pub fn router(state: Arc<ResourceStore>) -> axum::Router {
    routes::create_router(state)
}
