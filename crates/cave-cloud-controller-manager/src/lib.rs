//! cave-cloud-controller-manager — Cloud provider integration.
//!
//! Implements the upstream `cloud-controller-manager` interface narrowly:
//! the Node, Service (load balancer), and Route controllers. Specific
//! cloud-provider implementations (AWS, GCP, Azure) are out-of-scope —
//! cave ships a `MockProvider` adequate for testing the reconcile loops.

pub mod controllers;
pub mod models;
pub mod provider;
pub mod routes;
pub mod state;

use state::CloudControllerState;
use std::sync::Arc;

pub fn new_state() -> Arc<CloudControllerState> {
    Arc::new(CloudControllerState::new())
}

pub fn router(state: Arc<CloudControllerState>) -> axum::Router {
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
    fn parity_is_full_for_cloud_controller_manager_narrow_surface() {
        let r = calculate_parity().expect("manifest must parse");
        eprintln!(
            "cave-cloud-controller-manager parity: file={}/{} fn={}/{} test={}/{} surface={}/{} stubs={} overall={:.2}",
            r.file_parity.matched, r.file_parity.total,
            r.function_parity.matched, r.function_parity.total,
            r.test_parity.matched, r.test_parity.total,
            r.surface_parity.matched, r.surface_parity.total,
            r.stubs_detected, r.overall,
        );
        for g in &r.gaps {
            eprintln!("  gap: {:?} {} → {:?}", g.kind, g.upstream, g.local);
        }
        assert_eq!(r.function_parity.matched, r.function_parity.total);
        assert_eq!(r.surface_parity.matched, r.surface_parity.total);
        assert_eq!(r.test_parity.matched, r.test_parity.total);
        assert_eq!(r.file_parity.matched, r.file_parity.total);
        assert_eq!(r.stubs_detected, 0, "no stub macros allowed in source");
        assert!((r.overall - 1.0).abs() < 1e-5, "overall parity must be 1.0, got {}", r.overall);
    }
}
