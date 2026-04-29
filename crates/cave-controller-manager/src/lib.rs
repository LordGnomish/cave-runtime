//! cave-controller-manager — kube-controller-manager reimplementation.
//!
//! Hosts the built-in controllers that reconcile cluster state. Each controller
//! observes a subset of K8s resources and drives them toward a desired state.
//!
//! ## Hosted controllers
//!
//! - **deployment** — reconciles Deployment → ReplicaSet
//! - **replicaset** — reconciles ReplicaSet → Pods
//! - **node-lifecycle** — marks unhealthy Nodes NotReady, evicts pods
//! - **namespace** — drives Terminating Namespaces through finalizers
//! - **garbage-collector** — deletes resources whose owners are gone
//! - **endpoints** — fans out Services to backing Pod IPs
//! - **service-account** — provisions default SAs + tokens
//! - **job** — tracks Job completions and Pod lifecycle for batch workloads
//!
//! ## API
//!
//! ```text
//! GET  /api/controller-manager/health
//! GET  /api/controller-manager/metrics
//! GET  /api/controller-manager/controllers
//! POST /api/controller-manager/controllers/{name}/reconcile
//! GET  /api/controller-manager/controllers/{name}/status
//! ```

pub mod controllers;
pub mod manager;
pub mod models;
pub mod routes;

use manager::ControllerManager;
use std::sync::Arc;

/// Initialize controller-manager state with the default set of controllers.
pub fn new_state() -> Arc<ControllerManager> {
    Arc::new(manager::new_manager())
}

/// Build the axum router that exposes the manager's HTTP surface.
pub fn router(state: Arc<ControllerManager>) -> axum::Router {
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
