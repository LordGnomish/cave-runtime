//! cave-kubelet — Node agent.
//!
//! Watches the API server for pod assignments, manages container lifecycle
//! via cave-cri, and reports node status back to the control plane.
//!
//! ## API
//!
//! ```text
//! GET  /api/kubelet/health         — health check
//! GET  /api/kubelet/status         — node status report
//! GET  /api/kubelet/pods           — list managed pods
//! POST /api/kubelet/pods           — assign pod to this node
//! POST /api/kubelet/pods/{uid}/start
//! POST /api/kubelet/pods/{uid}/stop
//! DELETE /api/kubelet/pods/{uid}   — remove pod
//! ```

pub mod models;
pub mod agent;
pub mod routes;
pub mod csi;
pub mod probe;
pub mod eviction;

use agent::KubeletState;
use std::sync::Arc;

pub fn new_state() -> Arc<KubeletState> {
    Arc::new(KubeletState::default())
}

pub fn router(state: Arc<KubeletState>) -> axum::Router {
    routes::create_router(state)
}
