//! cave-cloud-controller-manager — cloud-provider-specific controllers.
//!
//! Hosts the cloud controllers that were split out of kube-controller-manager:
//!
//! - **node** — initializes Node objects with provider-supplied metadata
//!   (instance type, zone, addresses) and tears them down on instance deletion
//! - **route** — reconciles VPC routes for Pod CIDRs
//! - **service-lb** — provisions a cloud load-balancer for type=LoadBalancer Services
//! - **node-ipam** — allocates per-node Pod CIDRs from the cluster pool
//!
//! All controllers depend only on the [`provider::CloudProvider`] trait so a
//! single binary can target any cloud by swapping the provider implementation.
//!
//! ## API
//!
//! ```text
//! GET  /api/cloud-controller-manager/health
//! GET  /api/cloud-controller-manager/metrics
//! GET  /api/cloud-controller-manager/controllers
//! POST /api/cloud-controller-manager/controllers/{name}/reconcile
//! GET  /api/cloud-controller-manager/provider
//! ```

pub mod controllers;
pub mod manager;
pub mod models;
pub mod provider;
pub mod routes;

use manager::CloudControllerManager;
use std::sync::Arc;

pub fn new_state() -> Arc<CloudControllerManager> {
    Arc::new(manager::new_manager())
}

pub fn router(state: Arc<CloudControllerManager>) -> axum::Router {
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
