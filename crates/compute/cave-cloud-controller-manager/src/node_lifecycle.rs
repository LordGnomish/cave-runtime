// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Node lifecycle controller — condition reconciliation, heartbeat
//! freshness, eviction taint TTL.
//!
//! Mirrors `staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go`
//! plus the upstream `pkg/controller/nodelifecycle` lease evaluation.
//!
//! The cloud-provider half of the lifecycle controller is responsible for:
//!
//! 1. Watching `Node.status.conditions` for staleness — when the kubelet
//!    heartbeat goes silent, it flips `NodeReady` to `Unknown` and applies
//!    `node.kubernetes.io/unreachable:NoExecute` with a TTL.
//! 2. Lifting the unreachable taint when heartbeats resume.
//! 3. Honouring pod-level `tolerationSeconds` so pods aren't evicted faster
//!    than their tolerations allow.

use crate::node_controller::{
    NOT_READY_TAINT_KEY, OUT_OF_SERVICE_TAINT_KEY, UNREACHABLE_TAINT_KEY,
};
use crate::types::{Cite, CloudError};
use serde::{Deserialize, Serialize};

/// Mirrors `core/v1.NodeConditionType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeConditionType {
    Ready,
    MemoryPressure,
    DiskPressure,
    PIDPressure,
    NetworkUnavailable,
}

impl NodeConditionType {
    pub const fn key(self) -> &'static str {
        match self {
            NodeConditionType::Ready => "Ready",
            NodeConditionType::MemoryPressure => "MemoryPressure",
            NodeConditionType::DiskPressure => "DiskPressure",
            NodeConditionType::PIDPressure => "PIDPressure",
            NodeConditionType::NetworkUnavailable => "NetworkUnavailable",
        }
    }
}

/// Mirrors `core/v1.ConditionStatus` — `True` / `False` / `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}

impl ConditionStatus {
    pub const fn key(self) -> &'static str {
        match self {
            ConditionStatus::True => "True",
            ConditionStatus::False => "False",
            ConditionStatus::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCondition {
    pub kind: NodeConditionType,
    pub status: ConditionStatus,
    /// Seconds since the last heartbeat for this condition. Mirrors
    /// `lastHeartbeatTime` upstream after `time.Since` is applied.
    pub last_heartbeat_age_seconds: u64,
    /// Seconds since the condition last *transitioned* to its current
    /// status — used to skip eviction for newly-Unknown nodes.
    pub last_transition_age_seconds: u64,
}

impl NodeCondition {
    pub fn ready_true(age: u64) -> Self {
        Self {
            kind: NodeConditionType::Ready,
            status: ConditionStatus::True,
            last_heartbeat_age_seconds: age,
            last_transition_age_seconds: age,
        }
    }
}

/// Default heartbeat-staleness threshold. Upstream's
/// `--node-monitor-grace-period` is 40s; the lease-based path adds a
/// further `--node-monitor-period * 5` window before flipping to Unknown.
pub const HEARTBEAT_STALE_SECONDS: u64 = 40;

/// Default eviction-after-Unknown TTL. Upstream defaults to 5 minutes
/// (`--pod-eviction-timeout`).
pub const EVICTION_TTL_SECONDS: u64 = 300;

/// Time after which a `not-ready` taint is escalated to `unreachable`.
/// Mirrors upstream's `--node-monitor-grace-period` plus a small slack.
pub const NOT_READY_TO_UNREACHABLE_SECONDS: u64 = 60;

/// True iff the node's `Ready` condition heartbeat is older than the
/// staleness threshold. Mirrors `monitorNodeHealth` upstream.
pub fn is_heartbeat_stale(ready: &NodeCondition, threshold_seconds: u64) -> bool {
    ready.kind == NodeConditionType::Ready && ready.last_heartbeat_age_seconds > threshold_seconds
}

/// Compute the taint that *should* be applied for a node's current
/// `Ready` condition. Mirrors `markNodeAsNotReady` + `markNodeAsUnreachable`.
pub fn ready_condition_taint(ready: &NodeCondition) -> Option<&'static str> {
    match ready.status {
        ConditionStatus::True => None,
        ConditionStatus::False => Some(NOT_READY_TAINT_KEY),
        ConditionStatus::Unknown => Some(UNREACHABLE_TAINT_KEY),
    }
}

/// True iff a `True`-state pressure condition warrants the
/// out-of-service taint. Mirrors the kubelet "node out of service"
/// signalling introduced in v1.28.
pub fn is_out_of_service(conditions: &[NodeCondition]) -> bool {
    conditions.iter().any(|c| {
        matches!(
            c.kind,
            NodeConditionType::MemoryPressure
                | NodeConditionType::DiskPressure
                | NodeConditionType::PIDPressure
        ) && c.status == ConditionStatus::True
    })
}

/// Pick the next out-of-service taint write for `conditions`. Returns
/// `(add, remove)` where `add` is `Some(key)` when the taint should be
/// applied and `remove` is true when it should be cleared.
pub fn out_of_service_taint_write(
    conditions: &[NodeCondition],
    currently_tainted: bool,
) -> (Option<&'static str>, bool) {
    let want = is_out_of_service(conditions);
    match (want, currently_tainted) {
        (true, false) => (Some(OUT_OF_SERVICE_TAINT_KEY), false),
        (false, true) => (None, true),
        _ => (None, false),
    }
}

// ─── Eviction TTL ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvictionDecision {
    /// Pod stays; its toleration is not yet exhausted.
    Tolerate,
    /// Pod is evicted; toleration expired.
    Evict,
    /// Node became unreachable but the eviction grace period has not
    /// elapsed yet — wait.
    Wait,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodToleration {
    /// Pod's `tolerationSeconds` for the unreachable taint, if set.
    /// `None` means tolerate forever.
    pub toleration_seconds: Option<u64>,
}

impl PodToleration {
    pub fn forever() -> Self {
        Self {
            toleration_seconds: None,
        }
    }
    pub fn for_seconds(s: u64) -> Self {
        Self {
            toleration_seconds: Some(s),
        }
    }
}

/// Decide what to do with a pod when its node becomes unreachable.
/// `since_unreachable_seconds` is how long the node has been unreachable.
pub fn evaluate_eviction(
    tol: &PodToleration,
    since_unreachable_seconds: u64,
    grace_period_seconds: u64,
) -> EvictionDecision {
    match tol.toleration_seconds {
        None => EvictionDecision::Tolerate,
        Some(t) => {
            if since_unreachable_seconds < grace_period_seconds {
                EvictionDecision::Wait
            } else if since_unreachable_seconds >= t {
                EvictionDecision::Evict
            } else {
                EvictionDecision::Tolerate
            }
        }
    }
}

/// Count pods to be evicted out of `pods`. Mirrors the loop in
/// `nodeLifecycleController.processTaintBaseEviction`.
pub fn count_evictions(
    pods: &[PodToleration],
    since_unreachable_seconds: u64,
    grace_period_seconds: u64,
) -> u32 {
    pods.iter()
        .filter(|p| {
            evaluate_eviction(p, since_unreachable_seconds, grace_period_seconds)
                == EvictionDecision::Evict
        })
        .count() as u32
}

// ─── Monitor decision ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorOutcome {
    pub apply_taint: Option<&'static str>,
    pub remove_taint: Option<&'static str>,
    /// `True` when the cloud lifecycle controller should escalate to
    /// deletion (cloud confirmed the instance is gone).
    pub delete_node: bool,
}

impl MonitorOutcome {
    pub const fn noop() -> Self {
        Self {
            apply_taint: None,
            remove_taint: None,
            delete_node: false,
        }
    }
}

/// Given a node's `Ready` condition snapshot, return the taint write
/// the controller should perform. Mirrors `monitorNodeHealth`.
pub fn monitor_node(
    ready: &NodeCondition,
    currently_tainted: Option<&str>,
    threshold_seconds: u64,
) -> MonitorOutcome {
    let want = if is_heartbeat_stale(ready, threshold_seconds) {
        Some(UNREACHABLE_TAINT_KEY)
    } else {
        ready_condition_taint(ready)
    };
    match (want, currently_tainted) {
        (None, None) => MonitorOutcome::noop(),
        (None, Some(_)) => MonitorOutcome {
            apply_taint: None,
            remove_taint: currently_tainted.map(|k| match k {
                k if k == UNREACHABLE_TAINT_KEY => UNREACHABLE_TAINT_KEY,
                _ => NOT_READY_TAINT_KEY,
            }),
            delete_node: false,
        },
        (Some(key), None) => MonitorOutcome {
            apply_taint: Some(key),
            remove_taint: None,
            delete_node: false,
        },
        (Some(want_key), Some(have_key)) if want_key == have_key => MonitorOutcome::noop(),
        (Some(want_key), Some(have_key)) => MonitorOutcome {
            apply_taint: Some(want_key),
            remove_taint: Some(if have_key == UNREACHABLE_TAINT_KEY {
                UNREACHABLE_TAINT_KEY
            } else {
                NOT_READY_TAINT_KEY
            }),
            delete_node: false,
        },
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::k8s(
    "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
    "Controller",
);

/// Validate a heartbeat threshold — upstream caps the grace period at
/// 24 hours (`--node-monitor-grace-period`).
pub fn validate_heartbeat_threshold(seconds: u64) -> Result<(), CloudError> {
    if !(10..=86_400).contains(&seconds) {
        return Err(CloudError::InvalidConfig {
            provider: crate::types::ProviderName::Hetzner,
            reason: format!("heartbeat threshold {seconds} outside [10, 86400] s"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ready(status: ConditionStatus, hb_age: u64) -> NodeCondition {
        NodeCondition {
            kind: NodeConditionType::Ready,
            status,
            last_heartbeat_age_seconds: hb_age,
            last_transition_age_seconds: hb_age,
        }
    }

    // ─── Constants ───────────────────────────────────────────────────────────

    #[test]
    fn condition_type_keys_match_upstream() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "NodeConditionType",
            "tenant-cond-keys"
        );
        assert_eq!(NodeConditionType::Ready.key(), "Ready");
        assert_eq!(NodeConditionType::MemoryPressure.key(), "MemoryPressure");
        assert_eq!(NodeConditionType::DiskPressure.key(), "DiskPressure");
        assert_eq!(NodeConditionType::PIDPressure.key(), "PIDPressure");
        assert_eq!(
            NodeConditionType::NetworkUnavailable.key(),
            "NetworkUnavailable"
        );
    }

    #[test]
    fn condition_status_keys_match_upstream() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "ConditionStatus",
            "tenant-cstat-keys"
        );
        assert_eq!(ConditionStatus::True.key(), "True");
        assert_eq!(ConditionStatus::False.key(), "False");
        assert_eq!(ConditionStatus::Unknown.key(), "Unknown");
    }

    #[test]
    fn default_heartbeat_thresholds_match_upstream_defaults() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "DefaultNodeMonitorGracePeriod",
            "tenant-defaults"
        );
        assert_eq!(HEARTBEAT_STALE_SECONDS, 40);
        assert_eq!(EVICTION_TTL_SECONDS, 300);
        assert_eq!(NOT_READY_TO_UNREACHABLE_SECONDS, 60);
    }

    // ─── Heartbeat staleness ─────────────────────────────────────────────────

    #[test]
    fn fresh_heartbeat_is_not_stale() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-hb-fresh"
        );
        assert!(!is_heartbeat_stale(
            &ready(ConditionStatus::True, 5),
            HEARTBEAT_STALE_SECONDS
        ));
    }

    #[test]
    fn old_heartbeat_is_flagged_stale() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-hb-stale"
        );
        assert!(is_heartbeat_stale(
            &ready(ConditionStatus::True, 120),
            HEARTBEAT_STALE_SECONDS
        ));
    }

    #[test]
    fn heartbeat_at_exact_threshold_is_not_stale() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-hb-exact"
        );
        assert!(!is_heartbeat_stale(
            &ready(ConditionStatus::True, 40),
            HEARTBEAT_STALE_SECONDS
        ));
    }

    #[test]
    fn validate_heartbeat_threshold_rejects_out_of_range() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "Validate",
            "tenant-hb-validate"
        );
        assert!(validate_heartbeat_threshold(5).is_err());
        assert!(validate_heartbeat_threshold(86_401).is_err());
        assert!(validate_heartbeat_threshold(40).is_ok());
    }

    // ─── Ready condition → taint ─────────────────────────────────────────────

    #[test]
    fn ready_true_implies_no_taint() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsReady",
            "tenant-ready-true"
        );
        assert!(ready_condition_taint(&ready(ConditionStatus::True, 0)).is_none());
    }

    #[test]
    fn ready_false_emits_not_ready_taint() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsNotReady",
            "tenant-ready-false"
        );
        assert_eq!(
            ready_condition_taint(&ready(ConditionStatus::False, 0)),
            Some(NOT_READY_TAINT_KEY)
        );
    }

    #[test]
    fn ready_unknown_emits_unreachable_taint() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsUnreachable",
            "tenant-ready-unk"
        );
        assert_eq!(
            ready_condition_taint(&ready(ConditionStatus::Unknown, 0)),
            Some(UNREACHABLE_TAINT_KEY)
        );
    }

    // ─── Out-of-service detection ────────────────────────────────────────────

    #[test]
    fn out_of_service_when_memory_pressure_is_true() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "evaluateNodeCondition",
            "tenant-oos-mem"
        );
        let conds = vec![NodeCondition {
            kind: NodeConditionType::MemoryPressure,
            status: ConditionStatus::True,
            last_heartbeat_age_seconds: 0,
            last_transition_age_seconds: 0,
        }];
        assert!(is_out_of_service(&conds));
    }

    #[test]
    fn out_of_service_when_disk_pressure_is_true() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "evaluateNodeCondition",
            "tenant-oos-disk"
        );
        let conds = vec![NodeCondition {
            kind: NodeConditionType::DiskPressure,
            status: ConditionStatus::True,
            last_heartbeat_age_seconds: 0,
            last_transition_age_seconds: 0,
        }];
        assert!(is_out_of_service(&conds));
    }

    #[test]
    fn no_out_of_service_for_false_pressure() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "evaluateNodeCondition",
            "tenant-oos-false"
        );
        let conds = vec![NodeCondition {
            kind: NodeConditionType::MemoryPressure,
            status: ConditionStatus::False,
            last_heartbeat_age_seconds: 0,
            last_transition_age_seconds: 0,
        }];
        assert!(!is_out_of_service(&conds));
    }

    #[test]
    fn out_of_service_taint_write_adds_when_pressure_arrives() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "applyOutOfServiceTaint",
            "tenant-oos-add"
        );
        let conds = vec![NodeCondition {
            kind: NodeConditionType::PIDPressure,
            status: ConditionStatus::True,
            last_heartbeat_age_seconds: 0,
            last_transition_age_seconds: 0,
        }];
        let (add, rm) = out_of_service_taint_write(&conds, false);
        assert_eq!(add, Some(OUT_OF_SERVICE_TAINT_KEY));
        assert!(!rm);
    }

    #[test]
    fn out_of_service_taint_write_removes_when_pressure_clears() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "applyOutOfServiceTaint",
            "tenant-oos-rm"
        );
        let (add, rm) = out_of_service_taint_write(&[], true);
        assert!(add.is_none());
        assert!(rm);
    }

    #[test]
    fn out_of_service_taint_write_is_noop_when_state_matches() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "applyOutOfServiceTaint",
            "tenant-oos-noop"
        );
        let (add, rm) = out_of_service_taint_write(&[], false);
        assert!(add.is_none());
        assert!(!rm);
    }

    // ─── Eviction logic ──────────────────────────────────────────────────────

    #[test]
    fn pod_with_no_toleration_seconds_is_tolerated_forever() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "processTaintBaseEviction",
            "tenant-evict-forever"
        );
        let d = evaluate_eviction(&PodToleration::forever(), 10_000, 0);
        assert_eq!(d, EvictionDecision::Tolerate);
    }

    #[test]
    fn pod_with_toleration_seconds_is_evicted_when_exceeded() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "processTaintBaseEviction",
            "tenant-evict-tol"
        );
        let d = evaluate_eviction(&PodToleration::for_seconds(60), 120, 0);
        assert_eq!(d, EvictionDecision::Evict);
    }

    #[test]
    fn pod_within_toleration_seconds_is_tolerated() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "processTaintBaseEviction",
            "tenant-evict-within"
        );
        let d = evaluate_eviction(&PodToleration::for_seconds(60), 30, 0);
        assert_eq!(d, EvictionDecision::Tolerate);
    }

    #[test]
    fn pod_within_grace_period_is_waited() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "processTaintBaseEviction",
            "tenant-evict-grace"
        );
        let d = evaluate_eviction(&PodToleration::for_seconds(0), 5, 30);
        assert_eq!(d, EvictionDecision::Wait);
    }

    #[test]
    fn count_evictions_sums_evict_decisions_only() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "processTaintBaseEviction",
            "tenant-evict-count"
        );
        let pods = vec![
            PodToleration::forever(),
            PodToleration::for_seconds(60),  // evicted
            PodToleration::for_seconds(600), // tolerated
            PodToleration::for_seconds(0),   // evicted (since unreachable >= 0)
        ];
        assert_eq!(count_evictions(&pods, 120, 0), 2);
    }

    #[test]
    fn eviction_decision_at_exact_toleration_evicts() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "processTaintBaseEviction",
            "tenant-evict-exact"
        );
        let d = evaluate_eviction(&PodToleration::for_seconds(60), 60, 0);
        assert_eq!(d, EvictionDecision::Evict);
    }

    // ─── monitor_node ────────────────────────────────────────────────────────

    #[test]
    fn monitor_fresh_ready_node_is_a_no_op() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-mon-noop"
        );
        let r = monitor_node(
            &ready(ConditionStatus::True, 5),
            None,
            HEARTBEAT_STALE_SECONDS,
        );
        assert_eq!(r, MonitorOutcome::noop());
    }

    #[test]
    fn monitor_stale_heartbeat_applies_unreachable_taint() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsUnreachable",
            "tenant-mon-unreach"
        );
        let r = monitor_node(
            &ready(ConditionStatus::True, 120),
            None,
            HEARTBEAT_STALE_SECONDS,
        );
        assert_eq!(r.apply_taint, Some(UNREACHABLE_TAINT_KEY));
    }

    #[test]
    fn monitor_recovery_clears_taint() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsReady",
            "tenant-mon-recover"
        );
        let r = monitor_node(
            &ready(ConditionStatus::True, 5),
            Some(UNREACHABLE_TAINT_KEY),
            HEARTBEAT_STALE_SECONDS,
        );
        assert_eq!(r.remove_taint, Some(UNREACHABLE_TAINT_KEY));
        assert!(r.apply_taint.is_none());
    }

    #[test]
    fn monitor_swaps_not_ready_for_unreachable_when_heartbeat_stales() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "markNodeAsUnreachable",
            "tenant-mon-swap"
        );
        let r = monitor_node(
            &ready(ConditionStatus::False, 120),
            Some(NOT_READY_TAINT_KEY),
            HEARTBEAT_STALE_SECONDS,
        );
        assert_eq!(r.apply_taint, Some(UNREACHABLE_TAINT_KEY));
        assert_eq!(r.remove_taint, Some(NOT_READY_TAINT_KEY));
    }

    #[test]
    fn monitor_idempotent_when_already_correctly_tainted() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/nodelifecycle/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-mon-idem"
        );
        let r = monitor_node(
            &ready(ConditionStatus::False, 5),
            Some(NOT_READY_TAINT_KEY),
            HEARTBEAT_STALE_SECONDS,
        );
        assert_eq!(r, MonitorOutcome::noop());
    }

    #[test]
    fn ready_true_constructor_carries_age() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "NodeCondition",
            "tenant-mon-ctor"
        );
        let c = NodeCondition::ready_true(7);
        assert_eq!(c.last_heartbeat_age_seconds, 7);
        assert_eq!(c.last_transition_age_seconds, 7);
        assert_eq!(c.status, ConditionStatus::True);
    }

    #[test]
    fn pod_toleration_constructors_round_trip() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "Toleration",
            "tenant-tol-ctor"
        );
        assert_eq!(PodToleration::forever().toleration_seconds, None);
        assert_eq!(PodToleration::for_seconds(60).toleration_seconds, Some(60));
    }
}
