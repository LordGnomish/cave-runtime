//! cave-controller-manager — parity scaffold for `kube-controller-manager`.
//!
//! Mirrors `k8s.io/kubernetes/pkg/controller` (pinned to
//! [`types::UPSTREAM_VERSION`]). Each submodule provides:
//!
//! * a `Spec` struct describing what the user asked for
//! * a `Status` struct describing observed state
//! * a `reconcile(spec, status, tenant)` function returning a
//!   [`types::Reconcile`] decision
//!
//! Many code paths are intentionally [`unimplemented!`] — this is a *scaffold*
//! that establishes the surface area, type shape, and per-tenant invariants so
//! the full controller bodies can be filled in incrementally without breaking
//! downstream consumers.
//!
//! Every test in this crate carries an upstream `Cite` and a `TenantId` (see
//! [`crate::test_ctx`]) to keep the parity audit trail explicit.

#![allow(clippy::needless_doctest_main)]

pub mod types;

/// sweep-002 F2-D: per-controller adoption of `cave_kernel::reconcile::run_reconciler`.
/// Exposes `run_<controller>` factories that bridge each pure
/// `reconcile(spec, status, tenant)` decision function onto the shared kernel
/// loop with bounded queue, configurable backoff, and cancellation support.
pub mod runtime;

pub mod cronjob;
pub mod daemonset;
pub mod deployment;
pub mod endpointslice;
pub mod hpa;
pub mod job;
pub mod pdb;
pub mod replicaset;
pub mod service;
pub mod statefulset;

/// 100-pct sprint M2: GarbageCollector — owner-reference graph + cascade
/// planning for foreground / background / orphan deletion modes.
pub mod gc;

/// 100-pct sprint M3: light-weight GC controllers — PodGC + TTLAfterFinished.
pub mod gc_lite;

/// 100-pct sprint M3: NodeLifecycle / NodeLease — node heartbeat + Ready
/// transition + taint-based eviction trigger.
pub mod node_lease;

/// 100-pct PUSH-HARD M8: Node lifecycle deeper — taints + zone-state
/// classifier + per-zone rate-limited eviction queue.
pub mod node_lifecycle;

/// 100-pct sprint M3: Root CA publisher — kube-root-ca.crt ConfigMap
/// propagation across active namespaces.
pub mod root_ca_publisher;

/// 100-pct sprint M4: ServiceAccount controller + token controller.
pub mod sa;

/// 100-pct sprint M4: CertificateSigningRequest signer.
pub mod csr_signer;

/// 100-pct PUSH-HARD M9: CSR signer deeper — expirationSeconds clamping,
/// denied-wins resolution, kubelet-serving + apiserver-client-kubelet
/// subject validation.
pub mod csr_signer_deeper;

/// 100-pct PUSH-HARD M15: CSR auto-approver — bootstrap + self-node-client
/// recognizers (sarapprove parity).
pub mod csr_auto_approver;

/// 100-pct PUSH-HARD M15: PEM block extractor used by CSR signer dispatch.
pub mod csr_pem;

/// 100-pct sprint M4: RBAC controllers (ClusterRole aggregation).
pub mod rbac;

/// 100-pct sprint M5: EndpointSlice topology-aware hints
/// (PreferClose / topology-mode Auto algorithm).
pub mod endpointslice_topology;

/// 100-pct PUSH-HARD M10: EndpointSlice multi-port slice allocator
/// + per-slice MaxEndpointsPerSlice cap.
pub mod endpointslice_multiport;

/// 100-pct sprint M5: PV/PVC binder + volume expansion state machine.
pub mod pv;

/// 100-pct PUSH-HARD M11: ResourceQuota controller (used-tracker + admission gate).
pub mod resource_quota;

/// 100-pct PUSH-HARD M11: NamespaceController finalizer state machine.
pub mod namespace_controller;

/// 100-pct PUSH-HARD M11: Bootstrap-token signer for the cluster-info ConfigMap.
pub mod bootstrap_signer;

/// 100-pct PUSH-HARD M13: NodeLease deeper — holder rotation, renewal cadence,
/// LeaseLock leader election step machine.
pub mod node_lease_deeper;

/// 100-pct PUSH-HARD M13: RootCA publisher deeper — mutation detection,
/// PEM bundle equality, owner-ref/finalizer preservation, terminating-namespace
/// behavior.
pub mod root_ca_deeper;

/// deeper-002 batch — manager loop wiring + per-controller deepening
/// (StatefulSet PVC state machine, DaemonSet rollout + tolerations,
/// indexed Job, real cron parser, EndpointSlice keying, clusterIP
/// allocator + LoadBalancer reconciler).
pub mod deeper;

pub use types::{Cite, ControllerError, Reconcile, TenantId, UPSTREAM_PKG, UPSTREAM_VERSION};

#[cfg(test)]
mod tests_crosscut;

// ── Admin surface used by cave-runtime portal/cavectl ────────────────────────

/// Stable list of controller loops this crate provides. Mirrors the upstream
/// `--controllers` flag of `kube-controller-manager`. Order matches the
/// `pkg/controller/*` package layout.
pub const CONTROLLERS: &[&str] = &[
    "deployment",
    "replicaset",
    "statefulset",
    "daemonset",
    "job",
    "cronjob",
    "hpa",
    "pdb",
    "endpointslice",
    "endpointslice-topology",
    "endpointslice-multiport",
    "service",
    "garbage-collector",
    "podgc",
    "ttl-after-finished",
    "node-lease",
    "node-lifecycle",
    "root-ca-publisher",
    "serviceaccount",
    "serviceaccount-token",
    "csr-signer",
    "csr-approver",
    "rbac-aggregation",
    "pv-binder",
    "pv-attach-detach",
    "pv-protection",
    "resource-quota",
    "namespace-controller",
    "bootstrap-signer",
];

/// Stable identifier of the in-process leader. We do not yet run a real
/// LeaseLock election (that's [`node_lease_deeper`]'s job for the kube-side
/// API); for the manager binary itself we report the pod identity that owns
/// the embedded reconciler loop.
pub fn leader_state(holder: &str) -> serde_json::Value {
    serde_json::json!({
        "holder_identity": holder,
        "lease_kind": "single-process-embedded",
        "controllers_active": CONTROLLERS.len(),
        "upstream_version": UPSTREAM_VERSION,
        "upstream_pkg": UPSTREAM_PKG,
    })
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
mod admin_surface_tests {
    use super::*;

    #[test]
    fn controllers_list_is_non_empty_and_unique() {
        assert!(!CONTROLLERS.is_empty());
        let mut seen = std::collections::HashSet::new();
        for c in CONTROLLERS {
            assert!(seen.insert(*c), "duplicate controller: {c}");
        }
    }

    #[test]
    fn controllers_list_includes_workload_core() {
        for must in ["deployment", "replicaset", "statefulset", "daemonset", "job", "cronjob"] {
            assert!(CONTROLLERS.contains(&must), "missing core controller: {must}");
        }
    }

    #[test]
    fn leader_state_carries_holder_and_version() {
        let v = leader_state("manager-0");
        assert_eq!(v["holder_identity"], "manager-0");
        assert_eq!(v["upstream_version"], UPSTREAM_VERSION);
        assert_eq!(v["controllers_active"], CONTROLLERS.len());
    }

    #[test]
    fn calculate_parity_succeeds_on_pinned_manifest() {
        let report = calculate_parity().expect("parity calculation must succeed");
        // Some files are mapped, even if the percentage is partial.
        assert!(report.surface_parity.total > 0 || report.file_parity.total > 0);
    }
}
