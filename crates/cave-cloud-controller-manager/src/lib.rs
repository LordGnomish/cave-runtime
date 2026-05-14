// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-cloud-controller-manager — parity scaffold for `cloud-controller-manager`.
//!
//! Mirrors the `staging/src/k8s.io/cloud-provider` tree of upstream Kubernetes
//! (pinned to [`types::UPSTREAM_VERSION`]), plus minimal scaffolds for two
//! out-of-tree providers:
//!
//! * **Hetzner Cloud** — `hetznercloud/hcloud-cloud-controller-manager`
//!   (pinned to [`providers::hetzner::PROVIDER_VERSION`]).
//! * **Microsoft Azure** — `kubernetes-sigs/cloud-provider-azure`
//!   (pinned to [`providers::azure::PROVIDER_VERSION`]).
//!
//! Multi-tenancy is a first-class concern: every [`provider::CloudConfig`]
//! carries a [`types::TenantId`] and providers MUST refuse to act on a
//! resource belonging to a different tenant.

#![allow(clippy::needless_doctest_main)]

pub mod types;
pub mod provider;
pub mod provider_runtime;

pub mod node_controller;
pub mod node_lifecycle;
pub mod route_controller;
pub mod route_ipam;
pub mod route_orchestrator;
pub mod service_controller;
pub mod service_extras;
pub mod service_lb_lifecycle;
pub mod service_topology;

pub mod providers;

pub use provider::{
    CloudConfig, CloudProvider, ClustersIface, InstancesIface, LoadBalancerIface, RoutesIface,
    ZonesIface,
};
pub use types::{Cite, CloudError, ProviderName, Reconcile, TenantId, UPSTREAM_VERSION};

#[cfg(test)]
mod tests_crosscut;

// ── Admin surface used by cave-runtime portal/cavectl ────────────────────────

/// Stable list of cloud-provider controller loops this crate exposes. Mirrors
/// the upstream `cmd/cloud-controller-manager` controller set.
pub const CLOUD_CONTROLLERS: &[&str] = &[
    "node",
    "node-lifecycle",
    "service",
    "service-lb-lifecycle",
    "service-extras",
    "service-topology",
    "route",
    "route-ipam",
    "route-orchestrator",
];

/// Stable list of cloud providers compiled into this binary. Order is
/// alphabetical for deterministic CLI/portal output.
pub const PROVIDERS: &[&str] = &[
    "azure",
    "hetzner",
];

/// Calculate parity against the local source tree at compile-time crate root.
pub fn calculate_parity() -> Result<cave_kernel::parity::ParityReport, String> {
    cave_kernel::parity::calculate_from_str(
        include_str!("../parity.manifest.toml"),
        env!("CARGO_MANIFEST_DIR"),
    )
    .map_err(|e| e.to_string())
}

/// Snapshot of cloud-provider state used by the portal health endpoint.
pub fn provider_snapshot() -> serde_json::Value {
    serde_json::json!({
        "controllers_active": CLOUD_CONTROLLERS.len(),
        "providers_compiled_in": PROVIDERS,
        "upstream_version": UPSTREAM_VERSION,
    })
}

#[cfg(test)]
mod admin_surface_tests {
    use super::*;

    #[test]
    fn cloud_controllers_unique_and_non_empty() {
        assert!(!CLOUD_CONTROLLERS.is_empty());
        let mut seen = std::collections::HashSet::new();
        for c in CLOUD_CONTROLLERS {
            assert!(seen.insert(*c), "duplicate cloud controller: {c}");
        }
    }

    #[test]
    fn cloud_controllers_include_node_service_route() {
        for must in ["node", "service", "route"] {
            assert!(CLOUD_CONTROLLERS.contains(&must), "missing core: {must}");
        }
    }

    #[test]
    fn providers_alphabetical_and_unique() {
        let mut sorted = PROVIDERS.to_vec();
        sorted.sort();
        assert_eq!(sorted, PROVIDERS.to_vec());
        let unique: std::collections::HashSet<_> = PROVIDERS.iter().collect();
        assert_eq!(unique.len(), PROVIDERS.len());
    }

    #[test]
    fn provider_snapshot_carries_version_and_count() {
        let v = provider_snapshot();
        assert_eq!(v["controllers_active"], CLOUD_CONTROLLERS.len());
        assert_eq!(v["upstream_version"], UPSTREAM_VERSION);
    }

    #[test]
    fn calculate_parity_succeeds_on_pinned_manifest() {
        let report = calculate_parity().expect("parity calculation must succeed");
        assert!(report.surface_parity.total > 0 || report.file_parity.total > 0);
    }
}
