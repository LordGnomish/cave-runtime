//! cave-cri — Container Runtime Interface.
//!
//! Reimplements containerd/crun functionality in Rust:
//! - Linux namespace isolation (pid, net, mnt, uts, ipc)
//! - cgroup v2 resource limits (cpu, memory, pids)
//! - OCI image pull from registries (Docker Hub, Harbor)
//! - Root filesystem assembly via overlayfs
//! - Full container lifecycle (create, start, stop, kill, delete)
//!
//! ## API — 42 endpoints (100% containerd CRI parity)
//!
//! ```text
//! GET    /api/cri/health
//! GET    /api/cri/version
//! GET    /api/cri/status
//! GET    /api/cri/stats
//! GET    /api/cri/events
//! GET    /api/cri/metrics
//!
//! POST   /api/cri/containers
//! GET    /api/cri/containers
//! GET    /api/cri/containers/{id}
//! PUT    /api/cri/containers/{id}
//! DELETE /api/cri/containers/{id}
//! POST   /api/cri/containers/{id}/start
//! POST   /api/cri/containers/{id}/stop
//! POST   /api/cri/containers/{id}/kill
//! POST   /api/cri/containers/{id}/pause
//! POST   /api/cri/containers/{id}/unpause
//! POST   /api/cri/containers/{id}/exec
//! POST   /api/cri/containers/{id}/attach
//! POST   /api/cri/containers/{id}/checkpoint
//! POST   /api/cri/containers/{id}/restore
//! GET    /api/cri/containers/{id}/logs
//! GET    /api/cri/containers/{id}/stats
//! GET    /api/cri/containers/{id}/processes
//!
//! POST   /api/cri/images/pull
//! GET    /api/cri/images
//! GET    /api/cri/images/{reference}
//! DELETE /api/cri/images/{reference}
//! POST   /api/cri/images/{reference}/tag
//! GET    /api/cri/images/{reference}/history
//!
//! POST   /api/cri/sandboxes
//! GET    /api/cri/sandboxes
//! GET    /api/cri/sandboxes/{id}
//! DELETE /api/cri/sandboxes/{id}
//! GET    /api/cri/sandboxes/{id}/stats
//!
//! POST   /api/cri/snapshots
//! GET    /api/cri/snapshots
//! DELETE /api/cri/snapshots/{id}
//! GET    /api/cri/snapshots/{id}/mounts
//! GET    /api/cri/snapshots/{id}/usage
//!
//! POST   /api/cri/network/attach
//! POST   /api/cri/network/detach
//! GET    /api/cri/network/status
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
pub mod state_machine;
pub mod oci_spec;
pub mod logs;
pub mod health;

use routes::CriState;
use store::{ContainerStore, ImageStore, SandboxStore, SnapshotStore};
use registry::RegistryClient;
use dashmap::DashMap;
use std::sync::Arc;
use std::path::PathBuf;
use tokio::sync::Mutex;

/// Initialize cave-cri state.
pub fn new_state() -> Arc<CriState> {
    let cache_dir = PathBuf::from("/var/lib/cave/images");
    Arc::new(CriState {
        containers: ContainerStore::new(),
        images: ImageStore::new(),
        registry: RegistryClient::new(cache_dir),
        sandboxes: SandboxStore::new(),
        snapshots: SnapshotStore::new(),
        events: Mutex::new(Vec::new()),
        network: DashMap::new(),
    })
}

/// Create the axum router for cave-cri.
pub fn router(state: Arc<CriState>) -> axum::Router {
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

#[cfg(test)]
mod parity_tests {
    use super::*;

    #[test]
    fn parity_is_full_for_cri_surface() {
        let r = calculate_parity().expect("manifest must parse");
        eprintln!(
            "cave-cri parity: file={}/{} fn={}/{} test={}/{} surface={}/{} stubs={} overall={:.2}",
            r.file_parity.matched, r.file_parity.total,
            r.function_parity.matched, r.function_parity.total,
            r.test_parity.matched, r.test_parity.total,
            r.surface_parity.matched, r.surface_parity.total,
            r.stubs_detected, r.overall,
        );
        for g in &r.gaps {
            eprintln!("  gap: {:?} {} → {:?}", g.kind, g.upstream, g.local);
        }
        assert_eq!(r.function_parity.matched, r.function_parity.total,
                   "every mapped CRI handler must exist as fn in routes.rs");
        assert_eq!(r.surface_parity.matched, r.surface_parity.total,
                   "every HTTP surface must literally appear in source");
        assert_eq!(r.test_parity.matched, r.test_parity.total,
                   "every mapped upstream test must have a Rust counterpart");
        assert_eq!(r.file_parity.matched, r.file_parity.total,
                   "every mapped local file must exist");
        assert_eq!(r.stubs_detected, 0, "no stub macros allowed in source");
        assert!((r.overall - 1.0).abs() < 1e-5, "overall parity must be 1.0, got {}", r.overall);
    }
}
