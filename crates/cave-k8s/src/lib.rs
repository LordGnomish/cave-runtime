// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-k8s — Kubernetes control-plane umbrella.
//!
//! cave-k8s is the unified entry point onto Cave Runtime's Kubernetes
//! control-plane substrate. It pulls the eight subsystem crates
//!
//!   * `cave-apiserver`               — REST API surface + admission
//!   * `cave-scheduler`               — pod scheduling (plugin framework)
//!   * `cave-kubelet`                 — node agent + container lifecycle
//!   * `cave-kube-proxy`              — Service / EndpointSlice datapath
//!   * `cave-controller-manager`      — built-in controller loops
//!   * `cave-cloud-controller-manager`— cloud-provider controllers
//!   * `cave-cri`                     — container runtime (containerd parity)
//!   * `cave-etcd`                    — distributed key-value store
//!
//! together behind a single `ControlPlane` facade and adds:
//!
//!   * PQC-ready ServiceAccount token signing (`pqc::HybridSigner` —
//!     ECDSA P-256 + Ed25519 + ML-DSA-65 envelope).
//!   * Resource-quota / namespace-lifecycle / GC coordination across
//!     subsystem boundaries.
//!   * OpenAPI v3 + Aggregator API discovery served from one router.
//!   * Built-in admission chain (NamespaceLifecycle + ServiceAccount +
//!     ResourceQuota + LimitRanger + PodSecurity + VAP).
//!   * `cavectl k8s …` integration helpers.
//!
//! Upstream parity target: `kubernetes/kubernetes v1.32.0`
//! (`70d3cc986aa8221cd1dfb1121852688902d3bf53`, Apache-2.0).

pub mod admission;
pub mod aggregator;
pub mod authn;
pub mod authz;
pub mod cgroup;
pub mod cluster;
pub mod crd;
pub mod discovery;
pub mod error;
pub mod eviction;
pub mod garbage_collector;
pub mod images;
pub mod kubelet_facade;
pub mod models;
pub mod networking;
pub mod observability_metrics;
pub mod openapi;
pub mod pqc;
pub mod probes;
pub mod proxy_facade;
pub mod quota;
pub mod resources;
pub mod routes;
pub mod scheduler_facade;
pub mod state;
pub mod storage;
pub mod vap;
pub mod workloads;

pub use cluster::{ClusterConfig, ClusterStatus, ControlPlane};
pub use error::Error;
pub use models::{
    BuiltinKind, ClusterPhase, ComponentHealth, ComponentName, NodeRole, ResourceRef,
};
pub use state::State;

/// Module name advertised on the cave-runtime API surface.
pub const MODULE_NAME: &str = "k8s";

/// Upstream parity pin — kubernetes/kubernetes v1.32.0.
pub const UPSTREAM_VERSION: &str = "v1.32.0";

/// Upstream commit SHA matching [`UPSTREAM_VERSION`].
pub const UPSTREAM_SHA: &str = "70d3cc986aa8221cd1dfb1121852688902d3bf53";

/// Build a fresh axum router serving the cave-k8s control-plane surface.
pub fn router(state: std::sync::Arc<State>) -> axum::Router {
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
