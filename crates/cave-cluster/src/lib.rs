// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE Cluster — Kubernetes cluster lifecycle management.
//!
//! Compatible with: Rancher, Cluster API, cluster management tooling
//!
//! Features:
//! - Cluster CRUD (create, scale, upgrade, delete)
//! - Node pool management (add, remove, scale, labels, taints)
//! - Kubernetes version management
//! - etcd backup/restore per cluster
//! - Cluster health monitoring
//! - kubeconfig generation
//! - RBAC bootstrap (admin, developer, viewer roles)
//! - Network policy defaults
//! - Cluster add-ons management
//! - Multi-tenancy (namespace per tenant, quotas, limits)

pub mod addons;
pub mod cluster;
pub mod error;
pub mod etcd;
pub mod health;
pub mod kubeconfig;
pub mod network;
pub mod nodepool;
pub mod rbac;
pub mod routes;
pub mod tenant;
pub mod version;

use axum::Router;
use std::sync::Arc;

pub use cluster::ClusterStore;
pub use error::{ClusterError, ClusterResult};

pub const MODULE_NAME: &str = "cluster";

/// Shared state for the cluster module.
pub struct ClusterState {
    pub store: Arc<ClusterStore>,
}

impl Default for ClusterState {
    fn default() -> Self {
        Self {
            store: Arc::new(ClusterStore::new()),
        }
    }
}

/// Build Axum router for cluster lifecycle API.
pub fn router(state: Arc<ClusterState>) -> Router {
    routes::create_router(state)
}
