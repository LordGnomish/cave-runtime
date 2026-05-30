// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Pod-state predicates — port of `pkg/utils/pod/scheduling.go` from
//! kubernetes-sigs/karpenter v1.12.1 (sha ed490e8). Apache-2.0 upstream.
//!
//! The provisioning and disruption controllers gate almost every decision on
//! these predicates: is a pod *active* (worth scheduling capacity for),
//! *provisionable* (the kube-scheduler gave up on it), *reschedulable* (move
//! it to new capacity), *drainable* / *disruptable* (safe to evict). They are
//! pure functions over the pod's status / spec / metadata, plus a UTC clock
//! (`now_unix`) for the time-based `do-not-disrupt` window and the
//! stuck-terminating buffer.
//!
//! Ported faithfully except the `events.Recorder` emission inside
//! `IsDoNotDisruptActive` — those `recorder.Publish(...)` calls are
//! observability side effects that do not change any return value, so they are
//! omitted. The `corev1.Pod` shape is reduced to exactly the fields these
//! predicates read.

use std::collections::BTreeMap;

use crate::duration::parse_duration;
use crate::scheduling::taints::{Effect, Taint, Toleration};

/// `karpenter.sh/do-not-disrupt` pod annotation key.
pub const DO_NOT_DISRUPT_ANNOTATION_KEY: &str = "karpenter.sh/do-not-disrupt";
/// One minute, in seconds — the stuck-terminating buffer.
const STUCK_TERMINATING_BUFFER_SECS: i64 = 60;

/// `corev1.PodPhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PodPhase {
    #[default]
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

/// A reduced `corev1.PodCondition` (only the fields the predicates read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodCondition {
    pub type_: String,
    pub reason: String,
}

/// A reduced `metav1.OwnerReference` (apiVersion + kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerReference {
    pub api_version: String,
    pub kind: String,
}

/// `OwnerReference` constructor mirroring a GVK's `apiVersion`/`kind`.
pub fn owner(api_version: &str, kind: &str) -> OwnerReference {
    OwnerReference {
        api_version: api_version.to_string(),
        kind: kind.to_string(),
    }
}

/// The reduced `corev1.Pod` these predicates operate on. Timestamps are UTC
/// Unix seconds.
#[derive(Debug, Clone, Default)]
pub struct Pod {
    pub phase: PodPhase,
    /// `DeletionTimestamp` — `Some` once the pod is terminating.
    pub deletion_timestamp: Option<i64>,
    pub conditions: Vec<PodCondition>,
    /// `Spec.NodeName` — empty until bound.
    pub node_name: String,
    /// `Status.NominatedNodeName` — set while preempting.
    pub nominated_node_name: String,
    pub owner_references: Vec<OwnerReference>,
    pub annotations: BTreeMap<String, String>,
    /// `Status.StartTime`.
    pub start_time: Option<i64>,
    pub tolerations: Vec<Toleration>,
    /// `Spec.Affinity.PodAntiAffinity.RequiredDuringScheduling...` non-empty.
    pub has_required_pod_anti_affinity: bool,
    /// `Spec.Affinity.PodAntiAffinity.PreferredDuringScheduling...` non-empty.
    pub has_preferred_pod_anti_affinity: bool,
    /// Any (init or regular) container consumes a `ResourceClaim`.
    pub has_resource_claims: bool,
}

// ── basic state ──────────────────────────────────────────────────────────────

/// `IsTerminal`: the pod is in a terminal phase (Failed or Succeeded).
pub fn is_terminal(pod: &Pod) -> bool {
    pod.phase == PodPhase::Failed || pod.phase == PodPhase::Succeeded
}

/// `IsTerminating`: the pod has a deletion timestamp.
pub fn is_terminating(pod: &Pod) -> bool {
    pod.deletion_timestamp.is_some()
}

/// `IsActive`: not terminal and not terminating.
pub fn is_active(pod: &Pod) -> bool {
    !is_terminal(pod) && !is_terminating(pod)
}

/// `IsScheduled`: bound to a node.
pub fn is_scheduled(pod: &Pod) -> bool {
    !pod.node_name.is_empty()
}

/// `IsPreempting`: has a nominated node (about to schedule by preemption).
pub fn is_preempting(pod: &Pod) -> bool {
    !pod.nominated_node_name.is_empty()
}

/// `FailedToSchedule`: the kube-scheduler marked `PodScheduled=Unschedulable`.
pub fn failed_to_schedule(pod: &Pod) -> bool {
    pod.conditions
        .iter()
        .any(|c| c.type_ == "PodScheduled" && c.reason == "Unschedulable")
}

/// `IsStuckTerminating`: terminating for longer than the 1-minute buffer.
pub fn is_stuck_terminating(pod: &Pod, now_unix: i64) -> bool {
    match pod.deletion_timestamp {
        Some(ts) => now_unix - ts > STUCK_TERMINATING_BUFFER_SECS,
        None => false,
    }
}

// ── ownership ────────────────────────────────────────────────────────────────

/// `IsOwnedBy`: the pod has an owner reference matching any `(apiVersion, kind)`.
pub fn is_owned_by(pod: &Pod, gvks: &[(&str, &str)]) -> bool {
    gvks.iter().any(|(api_version, kind)| {
        pod.owner_references
            .iter()
            .any(|o| o.api_version == *api_version && o.kind == *kind)
    })
}

/// `IsOwnedByStatefulSet`.
pub fn is_owned_by_stateful_set(pod: &Pod) -> bool {
    is_owned_by(pod, &[("apps/v1", "StatefulSet")])
}

/// `IsOwnedByDaemonSet`.
pub fn is_owned_by_daemon_set(pod: &Pod) -> bool {
    is_owned_by(pod, &[("apps/v1", "DaemonSet")])
}

/// `IsOwnedByNode`: a static (mirror) pod owned by a Node.
pub fn is_owned_by_node(pod: &Pod) -> bool {
    is_owned_by(pod, &[("v1", "Node")])
}

// ── composite scheduling predicates ──────────────────────────────────────────

/// `IsProvisionable`: unschedulable, unbound, not preempting, not owned by a
/// DaemonSet or Node.
pub fn is_provisionable(pod: &Pod) -> bool {
    failed_to_schedule(pod)
        && !is_scheduled(pod)
        && !is_preempting(pod)
        && !is_owned_by_daemon_set(pod)
        && !is_owned_by_node(pod)
}

/// `IsReschedulable`: active (or a terminating StatefulSet pod), not owned by a
/// DaemonSet or Node.
pub fn is_reschedulable(pod: &Pod) -> bool {
    (is_active(pod) || (is_owned_by_stateful_set(pod) && is_terminating(pod)))
        && !is_owned_by_daemon_set(pod)
        && !is_owned_by_node(pod)
}

/// `IsPodEligibleForForcedEviction`: terminating, with a deletion timestamp
/// after the node's grace-period expiration.
pub fn is_pod_eligible_for_forced_eviction(
    pod: &Pod,
    node_grace_period_expiration: Option<i64>,
) -> bool {
    match (node_grace_period_expiration, pod.deletion_timestamp) {
        (Some(grace), Some(deletion)) => is_terminating(pod) && deletion > grace,
        _ => false,
    }
}

// ── do-not-disrupt / disruptable / drainable ─────────────────────────────────

/// `parseDoNotDisrupt`: parse a positive Go duration (nanoseconds). Returns
/// `None` for invalid or non-positive values.
fn parse_do_not_disrupt(value: &str) -> Option<i64> {
    match parse_duration(value) {
        Ok(d) if d > 0 => Some(d),
        _ => None,
    }
}

/// `IsDoNotDisruptActive`: whether the `do-not-disrupt` protection still holds.
/// `"true"` is unconditional; a duration value protects until the pod has run
/// that long (`podAge < duration`). Invalid values are treated as absent. The
/// upstream `events.Recorder` emission is omitted (non-behavioral).
pub fn is_do_not_disrupt_active(pod: &Pod, now_unix: i64) -> bool {
    let value = match pod.annotations.get(DO_NOT_DISRUPT_ANNOTATION_KEY) {
        Some(v) => v,
        None => return false,
    };
    if value == "true" {
        return true;
    }
    let duration = match parse_do_not_disrupt(value) {
        Some(d) => d,
        None => return false,
    };
    // Fail safe: without a start time, treat protection as active.
    let start = match pod.start_time {
        Some(s) => s,
        None => return true,
    };
    let pod_age_ns = (now_unix - start) * 1_000_000_000;
    pod_age_ns < duration
}

/// `IsDisruptable`: a non-active pod is always disruptable; an active pod is
/// disruptable only when its `do-not-disrupt` protection is not active.
pub fn is_disruptable(pod: &Pod, now_unix: i64) -> bool {
    !is_active(pod) || !is_do_not_disrupt_active(pod, now_unix)
}

/// `ToleratesDisruptedNoScheduleTaint`: the pod tolerates
/// `karpenter.sh/disruption=disrupting:NoSchedule`.
pub fn tolerates_disrupted_no_schedule_taint(pod: &Pod) -> bool {
    let taint = Taint {
        key: "karpenter.sh/disruption".to_string(),
        value: Some("disrupting".to_string()),
        effect: Effect::NoSchedule,
    };
    pod.tolerations.iter().any(|t| t.tolerates_taint(&taint))
}

/// `IsDrainable`: doesn't tolerate the disrupted taint, isn't stuck
/// terminating, and isn't a Node-owned mirror pod.
pub fn is_drainable(pod: &Pod, now_unix: i64) -> bool {
    !tolerates_disrupted_no_schedule_taint(pod)
        && !is_stuck_terminating(pod, now_unix)
        && !is_owned_by_node(pod)
}

/// `IsWaitingEviction`: not terminal and drainable.
pub fn is_waiting_eviction(pod: &Pod, now_unix: i64) -> bool {
    !is_terminal(pod) && is_drainable(pod, now_unix)
}

// ── affinity / DRA ───────────────────────────────────────────────────────────

/// `HasPodAntiAffinity`: any required or preferred pod anti-affinity term.
pub fn has_pod_anti_affinity(pod: &Pod) -> bool {
    pod.has_required_pod_anti_affinity || pod.has_preferred_pod_anti_affinity
}

/// `HasRequiredPodAntiAffinity`: a required pod anti-affinity term.
pub fn has_required_pod_anti_affinity(pod: &Pod) -> bool {
    pod.has_required_pod_anti_affinity
}

/// `HasDRARequirements`: a container consumes a ResourceClaim.
pub fn has_dra_requirements(pod: &Pod) -> bool {
    pod.has_resource_claims
}
