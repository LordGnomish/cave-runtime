//! cave-net — eBPF-based pod networking.
//!
//! Reimplements Cilium CNI functionality:
//! - Pod IP allocation from CIDR
//! - Service discovery (ClusterIP routing)
//! - Network policy enforcement (default-deny, allow rules)
//! - Flow recording (Hubble-style observability)
//!
//! On Linux kernel 7.0+: uses eBPF programs for kernel-level enforcement.
//! On other platforms: userspace simulation for development.

pub mod models;
pub mod dataplane;
pub mod routes;

use dataplane::NetState;
use std::sync::Arc;

pub fn new_state() -> Arc<NetState> {
    Arc::new(NetState::new())
}

pub fn router(state: Arc<NetState>) -> axum::Router {
    routes::create_router(state)
}
