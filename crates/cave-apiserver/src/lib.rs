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

#[cfg(test)]
mod parity_tests {
    use super::*;

    #[test]
    fn parity_is_full_for_apiserver_crud_surface() {
        let r = calculate_parity().expect("manifest must parse");
        eprintln!(
            "cave-apiserver parity: file={}/{} fn={}/{} test={}/{} surface={}/{} stubs={} overall={:.2}",
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
                   "every mapped CRUD handler must exist as fn in routes.rs");
        assert_eq!(r.surface_parity.matched, r.surface_parity.total,
                   "every HTTP surface path must literally appear in source");
        assert_eq!(r.test_parity.matched, r.test_parity.total,
                   "every mapped upstream test must have a Rust counterpart");
        assert_eq!(r.file_parity.matched, r.file_parity.total,
                   "every mapped local file must exist");
        assert_eq!(r.stubs_detected, 0, "no stub macros allowed in source");
        assert!((r.overall - 1.0).abs() < 1e-5, "overall parity must be 1.0, got {}", r.overall);
    }
}
