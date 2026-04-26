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

/// deeper-002 batch — manager loop wiring + per-controller deepening
/// (StatefulSet PVC state machine, DaemonSet rollout + tolerations,
/// indexed Job, real cron parser, EndpointSlice keying, clusterIP
/// allocator + LoadBalancer reconciler).
pub mod deeper;

pub use types::{Cite, ControllerError, Reconcile, TenantId, UPSTREAM_PKG, UPSTREAM_VERSION};
