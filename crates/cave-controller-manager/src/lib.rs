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

/// 100-pct sprint M3: Root CA publisher — kube-root-ca.crt ConfigMap
/// propagation across active namespaces.
pub mod root_ca_publisher;

/// 100-pct sprint M4: ServiceAccount controller + token controller.
pub mod sa;

/// 100-pct sprint M4: CertificateSigningRequest signer.
pub mod csr_signer;

/// 100-pct sprint M4: RBAC controllers (ClusterRole aggregation).
pub mod rbac;

/// 100-pct sprint M5: EndpointSlice topology-aware hints
/// (PreferClose / topology-mode Auto algorithm).
pub mod endpointslice_topology;

/// 100-pct sprint M5: PV/PVC binder + volume expansion state machine.
pub mod pv;

/// deeper-002 batch — manager loop wiring + per-controller deepening
/// (StatefulSet PVC state machine, DaemonSet rollout + tolerations,
/// indexed Job, real cron parser, EndpointSlice keying, clusterIP
/// allocator + LoadBalancer reconciler).
pub mod deeper;

pub use types::{Cite, ControllerError, Reconcile, TenantId, UPSTREAM_PKG, UPSTREAM_VERSION};
