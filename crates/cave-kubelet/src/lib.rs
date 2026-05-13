//! cave-kubelet — Node agent.
//!
//! Watches the API server for pod assignments, manages container lifecycle
//! via cave-cri, and reports node status back to the control plane.
//!
//! ## API
//!
//! ```text
//! GET   /api/kubelet/health          — health check
//! GET   /api/kubelet/status          — node status report
//! GET   /api/kubelet/pods            — list managed pods
//! POST /api/kubelet/pods            — assign pod to this node
//! POST /api/kubelet/pods/{uid}/start
//! POST /api/kubelet/pods/{uid}/stop
//! DELETE /api/kubelet/pods/{uid}    — remove pod
//! ```

/// Re-export models for external use.
pub mod models;

/// Core agent logic for managing pod lifecycle.
pub mod agent;

/// HTTP route definitions for the kubelet API.
pub mod routes;

/// Container Storage Interface (CSI) integration.
pub mod csi;

/// Health and readiness probes.
pub mod probe;

/// Eviction policies for resource management.
pub mod eviction;

/// Streaming support for logs and exec.
pub mod streaming;

/// Security contexts and policies.
pub mod security;

/// AppArmor profile management.
pub mod apparmor;

/// Pod resources API implementation.
pub mod podresources;

/// Topology management utilities.
pub mod topology;

/// CPU manager implementation.
pub mod cpumanager;

/// Memory manager implementation.
pub mod memorymanager;

/// Device plugin integration.
pub mod deviceplugin;

/// Dynamic Resource Allocation (DRA) support.
pub mod dra;

/// DRA v1alpha2 specific implementations.
pub mod dra_v1alpha2;

/// Sidecar container management.
pub mod sidecar;

// deeper-003 — node-side runtime modules.
/// Container metrics collection and reporting.
pub mod container_metrics;

/// Image garbage collection logic.
pub mod image_gc;

/// Kubelet configuration handling.
pub mod kubelet_config;

/// Node lease management.
pub mod node_lease;

/// Plugin watcher for dynamic updates.
pub mod plugin_watcher;

/// Topology manager for resource placement.
pub mod topology_manager;

/// Volume reconciler for persistent volumes.
pub mod volume_reconciler;

// sweep-001 — node-side pod GC (mirrors upstream pkg/kubelet/pod/pod_gc.go).
/// Pod garbage collection logic.
pub mod pod_gc;

/// k8s-core push 2026-05-13: PodStatusManager — queue, lazy-enqueue
/// hash-dedupe, exponential-backoff retry, drop-on-delete semantics.
/// Mirrors `pkg/kubelet/status/status_manager.go`.
pub mod pod_status_manager;

/// k8s-core push 2026-05-13: prober worker pool + restart-coordination
/// ledger. Sits on top of `probe::ProberManager` to bound concurrency
/// and de-duplicate restart-trigger fan-out.
/// Mirrors `pkg/kubelet/prober/worker.go` + `prober/results/`.
pub mod prober;

/// k8s-core push batch2 2026-05-13: preStop / postStart hook
/// orchestration with per-hook timeouts. Mirrors `pkg/kubelet/lifecycle/`.
pub mod lifecycle;

/// k8s-core push batch2 2026-05-13: critical-pod preemption admit
/// handler. Selects lowest-priority pods to evict for a system-
/// critical incoming pod. Mirrors `pkg/kubelet/preemption/`.
pub mod preemption;

/// k8s-core push batch3 2026-05-13: graceful node-shutdown ordering
/// (KEP-2000). Mirrors `pkg/kubelet/nodeshutdown/`.
pub mod nodeshutdown;

/// k8s-core push batch3 2026-05-13: standalone runonce mode for
/// boot-time static-pod manifests. Mirrors `pkg/kubelet/runonce/`.
pub mod runonce;

/// k8s-core push batch3 2026-05-13: user-namespace allocator
/// (KEP-127). Mirrors `pkg/kubelet/userns/`.
pub mod userns;

/// k8s-core push batch3 2026-05-13: cgroup v2 backend abstraction +
/// path conventions for kubepods slices. Mirrors
/// `pkg/kubelet/cm/util/cgroups/`.
pub mod cgroups;

use agent::KubeletState;
use std::sync::Arc;

/// Creates a new default KubeletState wrapped in an Arc.
pub fn new_state() -> Arc<KubeletState> {
    Arc::new(KubeletState::default())
}

/// Creates the axum Router for the kubelet API using the provided state.
pub fn router(state: Arc<KubeletState>) -> axum::Router {
    routes::create_router(state)
}
