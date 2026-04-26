//! deeper-002 batch — real implementations for controller surface area
//! that the scaffold + deeper-001 left as thin reconcile shells.
//!
//! Pinned to k8s v1.36.0 ([`crate::types::UPSTREAM_VERSION`]). Modules:
//!
//! * [`manager`] — event source + workqueue + sync-controller plumbing.
//! * [`statefulset_pvc`] — ordered creation, PVC binding state machine,
//!   scale-down, set-deletion cascade.
//! * [`daemonset_rollout`] — rolling-update partition + taint/toleration
//!   evaluator (`pkg/util/tolerations`).
//! * [`job_indexed`] — indexed-job scheduler + per-index completion table.
//! * [`cronjob_parser`] — real 5-field cron expression parser +
//!   concurrencyPolicy state machine + suspend.
//! * [`endpointslice_keying`] — per-service slice allocator + hash-based
//!   slice keying (`MaxEndpointsPerSlice`).
//! * [`service_ip`] — clusterIP CIDR allocator + LoadBalancer reconciler
//!   with finalizer ordering.

pub mod cronjob_parser;
pub mod daemonset_rollout;
pub mod endpointslice_keying;
pub mod job_indexed;
pub mod manager;
pub mod service_ip;
pub mod statefulset_pvc;
