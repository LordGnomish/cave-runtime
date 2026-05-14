// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NodeLease controller — `pkg/controller/nodelifecycle/node_lifecycle_controller.go`.
//!
//! Each node renews a `coordination.k8s.io/Lease` named after itself in the
//! `kube-node-lease` namespace. The controller-manager watches these leases:
//!
//! * If `lease.spec.renewTime + lease_duration_sec` is in the past, the node
//!   is treated as "lease-expired".
//! * After lease expiry plus `node_monitor_grace_period`, the controller
//!   transitions `Node.Ready` to `Unknown` and applies the `NotReady` taint.
//! * After `pod_eviction_timeout`, taint-based eviction begins evicting pods.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub holder_identity: String,
    pub lease_duration_sec: u32,
    pub renew_time_sec: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeReadyState {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeAction {
    /// Lease still valid; nothing to do.
    Healthy,
    /// Lease has just expired; record an internal warning but don't change
    /// status yet.
    LeaseExpired,
    /// Lease expired AND grace period elapsed → mark Unknown + taint NotReady.
    MarkUnknown,
    /// Eviction timeout elapsed → start taint-based eviction of bound pods.
    StartEviction,
}

pub const DEFAULT_LEASE_DURATION_SEC: u32 = 40;
pub const DEFAULT_NODE_MONITOR_GRACE_PERIOD_SEC: u32 = 50;
pub const DEFAULT_POD_EVICTION_TIMEOUT_SEC: u32 = 5 * 60;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NodeMonitorConfig {
    pub lease_duration_sec: u32,
    pub monitor_grace_sec: u32,
    pub eviction_timeout_sec: u32,
}

impl Default for NodeMonitorConfig {
    fn default() -> Self {
        Self {
            lease_duration_sec: DEFAULT_LEASE_DURATION_SEC,
            monitor_grace_sec: DEFAULT_NODE_MONITOR_GRACE_PERIOD_SEC,
            eviction_timeout_sec: DEFAULT_POD_EVICTION_TIMEOUT_SEC,
        }
    }
}

/// Returns true when the lease has not been renewed within its duration.
pub fn is_lease_expired(lease: &Lease, now_sec: u64) -> bool {
    let expire = lease.renew_time_sec + lease.lease_duration_sec as u64;
    now_sec > expire
}

/// Decide what action to take on a node given its lease and timing config.
pub fn evaluate(
    lease: &Lease,
    cfg: &NodeMonitorConfig,
    now_sec: u64,
) -> NodeAction {
    let expire = lease.renew_time_sec + lease.lease_duration_sec as u64;
    if now_sec <= expire {
        return NodeAction::Healthy;
    }
    let elapsed_since_expiry = now_sec - expire;
    if elapsed_since_expiry < cfg.monitor_grace_sec as u64 {
        return NodeAction::LeaseExpired;
    }
    if elapsed_since_expiry < cfg.monitor_grace_sec as u64 + cfg.eviction_timeout_sec as u64 {
        return NodeAction::MarkUnknown;
    }
    NodeAction::StartEviction
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
    "Controller",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn lease(renew: u64, dur: u32) -> Lease {
        Lease {
            holder_identity: "node-a".into(),
            lease_duration_sec: dur,
            renew_time_sec: renew,
        }
    }

    #[test]
    fn fresh_lease_is_not_expired() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "tryUpdateNodeHealth",
            "tenant-nl-fresh"
        );
        assert!(!is_lease_expired(&lease(100, 40), 130));
    }

    #[test]
    fn lease_at_exact_expiry_not_yet_expired() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "tryUpdateNodeHealth",
            "tenant-nl-edge"
        );
        // expire = 100 + 40 = 140; at now=140 still valid (strictly after).
        assert!(!is_lease_expired(&lease(100, 40), 140));
    }

    #[test]
    fn lease_past_expiry_is_expired() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "tryUpdateNodeHealth",
            "tenant-nl-past"
        );
        assert!(is_lease_expired(&lease(100, 40), 200));
    }

    #[test]
    fn evaluate_returns_healthy_within_lease() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-nl-eval-healthy"
        );
        let cfg = NodeMonitorConfig::default();
        assert_eq!(evaluate(&lease(100, 40), &cfg, 130), NodeAction::Healthy);
    }

    #[test]
    fn evaluate_returns_lease_expired_within_grace() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-nl-eval-grace"
        );
        let cfg = NodeMonitorConfig::default();
        // expire=140, grace=50; now=160 → 20s past expiry, still in grace.
        assert_eq!(evaluate(&lease(100, 40), &cfg, 160), NodeAction::LeaseExpired);
    }

    #[test]
    fn evaluate_returns_mark_unknown_after_grace() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsNotReady",
            "tenant-nl-eval-mark-unknown"
        );
        let cfg = NodeMonitorConfig::default();
        // expire=140, grace=50 → 50s past expiry triggers MarkUnknown.
        assert_eq!(evaluate(&lease(100, 40), &cfg, 200), NodeAction::MarkUnknown);
    }

    #[test]
    fn evaluate_returns_start_eviction_after_eviction_timeout() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "evictPods",
            "tenant-nl-eval-eviction"
        );
        let cfg = NodeMonitorConfig::default();
        // expire=140; need elapsed >= grace(50) + eviction(300) = 350 → now >= 490.
        assert_eq!(evaluate(&lease(100, 40), &cfg, 600), NodeAction::StartEviction);
    }

    #[test]
    fn defaults_match_upstream_constants() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "defaults",
            "tenant-nl-defaults"
        );
        assert_eq!(DEFAULT_LEASE_DURATION_SEC, 40);
        assert_eq!(DEFAULT_NODE_MONITOR_GRACE_PERIOD_SEC, 50);
        assert_eq!(DEFAULT_POD_EVICTION_TIMEOUT_SEC, 300);
    }

    #[test]
    fn node_action_serializes_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/nodelifecycle/node_lifecycle_controller.go",
            "NodeAction",
            "tenant-nl-action-serde"
        );
        for a in [
            NodeAction::Healthy,
            NodeAction::LeaseExpired,
            NodeAction::MarkUnknown,
            NodeAction::StartEviction,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: NodeAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }

    #[test]
    fn node_ready_state_round_trips() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/types.go",
            "NodeConditionType",
            "tenant-nl-ready-serde"
        );
        for s in [NodeReadyState::True, NodeReadyState::False, NodeReadyState::Unknown] {
            let bytes = serde_json::to_string(&s).unwrap();
            let back: NodeReadyState = serde_json::from_str(&bytes).unwrap();
            assert_eq!(s, back);
        }
    }
}
