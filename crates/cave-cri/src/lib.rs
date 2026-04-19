//! cave-cri — Container Runtime Interface.
//!
//! Reimplements containerd/crun functionality in Rust:
//! - Linux namespace isolation (pid, net, mnt, uts, ipc)
//! - cgroup v2 resource limits (cpu, memory, pids)
//! - OCI image pull from registries (Docker Hub, Harbor)
//! - Root filesystem assembly via overlayfs
//! - Full container lifecycle (create, start, stop, kill, delete)
//!
//! ## API
//!
//! ```text
//! POST   /api/cri/containers           — create container
//! GET    /api/cri/containers           — list containers
//! GET    /api/cri/containers/{id}      — inspect
//! POST   /api/cri/containers/{id}/start
//! POST   /api/cri/containers/{id}/stop
//! POST   /api/cri/containers/{id}/kill
//! DELETE /api/cri/containers/{id}
//! POST   /api/cri/images/pull          — pull from registry
//! GET    /api/cri/images               — list local images
//! GET    /api/cri/health
//! ```

pub mod models;
pub mod error;
pub mod namespace;
pub mod cgroup;
pub mod registry;
pub mod rootfs;
pub mod runtime;
pub mod store;
pub mod routes;

use routes::CriState;
use store::{ContainerStore, ImageStore};
use registry::RegistryClient;
use std::sync::Arc;
use std::path::PathBuf;

/// Initialize cave-cri state.
pub fn new_state() -> Arc<CriState> {
    let cache_dir = PathBuf::from("/var/lib/cave/images");
    Arc::new(CriState {
        containers: ContainerStore::new(),
        images: ImageStore::new(),
        registry: RegistryClient::new(cache_dir),
    })
}

/// Create the axum router for cave-cri.
pub fn router(state: Arc<CriState>) -> axum::Router {
    routes::create_router(state)
}
