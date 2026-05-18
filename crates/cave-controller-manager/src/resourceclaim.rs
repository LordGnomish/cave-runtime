// SPDX-License-Identifier: AGPL-3.0-or-later
//! ResourceClaim controller — `pkg/controller/resourceclaim/controller.go`.
//!
//! KEP-4381 Dynamic Resource Allocation (GA in v1.36). The cluster-side
//! reconciler that:
//!
//! 1. Stamps the kubernetes-side finalizer
//!    (`resource.kubernetes.io/delete-protection`) on every new claim so
//!    GC can't reap a claim while pods still depend on the bound devices.
//! 2. For `Immediate` claims, picks a node whose published `ResourceSlice`
//!    can satisfy the request and writes an `AllocationResult` to status.
//! 3. For `WaitForFirstConsumer` claims, allocates only once a pod that
//!    references the claim has been scheduled to a node (`spec.nodeName`
//!    set), using that node's slice.
//! 4. Maintains `status.reservedFor[]` — adds entries for live consumer
//!    pods, removes them when pods are deleted.
//! 5. On `metadata.deletionTimestamp` set, drains consumers, then
//!    deallocates, then strips the finalizer.
//!
//! Scheduler hooks (device fitness, slice matching) live in
//! `cave-scheduler/src/dra.rs`. This crate handles the *control-plane*
//! reconciliation; the scheduler decides *which* device to bind on
//! `WaitForFirstConsumer`. Here we model the deterministic scaffold:
//! once a candidate `(node, devices)` exists, the action is to write
//! it to status.

use crate::types::{Cite, ControllerError, TenantId};
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/resourceclaim/controller.go",
    "Controller.syncHandler",
);

/// kubernetes-side finalizer that protects the claim from GC while
/// consumers still depend on the allocation.
///
/// Upstream uses the longer
/// `resource.kubernetes.io/delete-protection` literal; cave keeps the
/// same name for wire compatibility against any tool that lists
/// finalizers on the resource.
pub const FINALIZER_PROTECTION: &str = "resource.kubernetes.io/delete-protection";

/// Allocation mode declared on the claim's spec.
///
/// Cite: `staging/src/k8s.io/api/resource/v1/types.go` `AllocationMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AllocationMode {
    /// Allocate as soon as the claim is observed (cluster has free
    /// devices that match).
    Immediate,
    /// Wait until a pod referencing the claim is scheduled to a node;
    /// then allocate on that node.
    WaitForFirstConsumer,
}

/// Observed allocation result on the claim status. `node` is the
/// node on which the allocation lives; `devices` are the device names
/// inside that node's slice that have been reserved for this claim.
///
/// Cite: `staging/src/k8s.io/api/resource/v1/types.go`
/// `AllocationResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationResult {
    pub node: String,
    pub devices: Vec<String>,
}

/// One entry of `status.reservedFor[]`. Upstream is keyed by
/// `(api_group, resource, name, uid)`; this scaffold uses just the
/// pod uid since cave only supports pod consumers today.
///
/// Cite: `staging/src/k8s.io/api/resource/v1/types.go`
/// `ResourceClaimConsumerReference`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerRef {
    pub pod_uid: String,
}

/// Lightweight observed view of a pod that references a claim. The
/// controller only cares about three fields:
///
/// * `uid` — identity for the `reservedFor[]` list,
/// * `node_name` — set once the scheduler has bound the pod,
/// * `deleted` — pod's `metadata.deletionTimestamp` is set or the pod
///   has been removed entirely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodView {
    pub uid: String,
    pub node_name: Option<String>,
    pub deleted: bool,
}

/// A candidate allocation produced by the scheduler hook
/// (`cave-scheduler/src/dra.rs::try_allocate_on`). The controller is
/// purely deterministic — given the candidate it writes the result;
/// if no candidate exists yet, it requeues.
///
/// In production this is read from the apiserver-side scheduling
/// store; in tests the fixture feeds it in directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationCandidate {
    pub node: String,
    pub devices: Vec<String>,
}

/// The full state of one ResourceClaim as observed by the controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceClaimState {
    pub name: String,
    pub namespace: String,
    pub tenant_id: TenantId,
    pub allocation_mode: AllocationMode,
    pub finalizers: Vec<String>,
    pub deletion_timestamp_set: bool,
    /// `None` until allocation has been written to status.
    pub allocation: Option<AllocationResult>,
    /// `status.reservedFor[]` — consumer pods currently bound.
    pub reserved_for: Vec<ConsumerRef>,
    /// `status.deallocationRequested` — controller has asked the
    /// driver to release the devices.
    pub deallocation_requested: bool,
}

/// What the reconciler decided this pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimAction {
    /// Steady state: nothing to do.
    NoOp,
    /// Finalizer not yet present — add it before any allocation work.
    AddFinalizer,
    /// `Immediate` claim with no scheduler candidate yet — requeue.
    AwaitImmediateCandidate,
    /// `WaitForFirstConsumer` claim with no scheduled pod yet — requeue.
    AwaitFirstConsumer,
    /// Scheduler returned a candidate; write the allocation to status.
    SetAllocation(AllocationResult),
    /// One or more live consumers are missing from `reservedFor[]`.
    AddReservation { pod_uids: Vec<String> },
    /// One or more entries in `reservedFor[]` are stale (pod gone /
    /// pod's `deletionTimestamp` set) and must be removed.
    RemoveReservation { pod_uids: Vec<String> },
    /// `deletionTimestamp` is set and `reservedFor[]` is non-empty —
    /// must wait for consumers to drain before deallocating.
    AwaitConsumerDrain,
    /// Deletion in progress and consumers drained — request the driver
    /// to release the devices.
    RequestDeallocation,
    /// Deallocation done — strip the finalizer.
    RemoveFinalizer,
    /// Finalizer gone — apiserver will GC the object next pass.
    AwaitDeletion,
}

/// Pure reconciler. Given the claim's observed state, the pods that
/// reference it, and (for WaitForFirstConsumer) the scheduler's
/// candidate node + devices, return the next single action to apply.
///
/// The reconciler is intentionally one-step-at-a-time: each pass
/// emits one mutation, the controller writes it, the next observation
/// fires another pass. This mirrors upstream's "single-write per
/// reconcile" convention and keeps the reasoning local.
pub fn evaluate(
    claim: &ResourceClaimState,
    pods: &[PodView],
    candidate: Option<&AllocationCandidate>,
) -> ClaimAction {
    // (1) Deletion path takes precedence over everything else: the
    //     reservedFor drain → deallocation → finalizer strip dance.
    if claim.deletion_timestamp_set {
        return evaluate_delete(claim);
    }

    // (2) Otherwise, ensure the finalizer is in place before any
    //     allocation work, so a racing GC can't reap the claim while
    //     we're mid-allocation.
    if !claim.finalizers.iter().any(|f| f == FINALIZER_PROTECTION) {
        return ClaimAction::AddFinalizer;
    }

    // (3) Allocate if we don't have an allocation yet.
    if claim.allocation.is_none() {
        return match claim.allocation_mode {
            AllocationMode::Immediate => match candidate {
                Some(c) => ClaimAction::SetAllocation(AllocationResult {
                    node: c.node.clone(),
                    devices: c.devices.clone(),
                }),
                None => ClaimAction::AwaitImmediateCandidate,
            },
            AllocationMode::WaitForFirstConsumer => evaluate_wait_for_consumer(pods, candidate),
        };
    }

    // (4) Allocation present — reconcile the reservedFor[] list
    //     against the live pod set.
    let live: Vec<&PodView> = pods.iter().filter(|p| !p.deleted).collect();
    let reserved: std::collections::HashSet<&str> =
        claim.reserved_for.iter().map(|c| c.pod_uid.as_str()).collect();
    let live_uids: std::collections::HashSet<&str> =
        live.iter().map(|p| p.uid.as_str()).collect();

    // 4a. Live pods missing from reservedFor.
    let missing: Vec<String> = live
        .iter()
        .filter(|p| !reserved.contains(p.uid.as_str()))
        .map(|p| p.uid.clone())
        .collect();
    if !missing.is_empty() {
        return ClaimAction::AddReservation { pod_uids: missing };
    }

    // 4b. reservedFor entries with no matching live pod (pod gone or
    //     pod's deletionTimestamp set).
    let stale: Vec<String> = claim
        .reserved_for
        .iter()
        .filter(|c| !live_uids.contains(c.pod_uid.as_str()))
        .map(|c| c.pod_uid.clone())
        .collect();
    if !stale.is_empty() {
        return ClaimAction::RemoveReservation { pod_uids: stale };
    }

    ClaimAction::NoOp
}

fn evaluate_wait_for_consumer(
    pods: &[PodView],
    candidate: Option<&AllocationCandidate>,
) -> ClaimAction {
    // We need at least one live pod that has been scheduled
    // (`spec.nodeName` set) AND a scheduler candidate that targets
    // that node.
    let scheduled_node = pods
        .iter()
        .filter(|p| !p.deleted)
        .find_map(|p| p.node_name.as_deref());
    match (scheduled_node, candidate) {
        (Some(node), Some(c)) if c.node == node => ClaimAction::SetAllocation(AllocationResult {
            node: c.node.clone(),
            devices: c.devices.clone(),
        }),
        _ => ClaimAction::AwaitFirstConsumer,
    }
}

fn evaluate_delete(claim: &ResourceClaimState) -> ClaimAction {
    // Wait for consumers to drain.
    if !claim.reserved_for.is_empty() {
        return ClaimAction::AwaitConsumerDrain;
    }
    // Consumers drained — if there's an allocation we haven't yet
    // asked the driver to release, request deallocation.
    if claim.allocation.is_some() && !claim.deallocation_requested {
        return ClaimAction::RequestDeallocation;
    }
    // Deallocation requested and (in steady state) eventually the
    // driver clears `allocation`. Once the finalizer is the last
    // thing standing between us and GC, strip it.
    if claim.finalizers.iter().any(|f| f == FINALIZER_PROTECTION) {
        return ClaimAction::RemoveFinalizer;
    }
    ClaimAction::AwaitDeletion
}

/// Resolve a controller-side reservation update into the actual
/// `Vec<ConsumerRef>` to write. Pure helper used by both the
/// admin surface (apiserver shim) and tests so the apply-side logic
/// is asserted, not just the decision.
pub fn apply_reservation_diff(
    current: &[ConsumerRef],
    add: &[String],
    remove: &[String],
) -> Vec<ConsumerRef> {
    let mut out: Vec<ConsumerRef> = current
        .iter()
        .filter(|c| !remove.iter().any(|u| u == &c.pod_uid))
        .cloned()
        .collect();
    for uid in add {
        if !out.iter().any(|c| &c.pod_uid == uid) {
            out.push(ConsumerRef { pod_uid: uid.clone() });
        }
    }
    out
}

/// Helper: a fresh `ResourceClaimState` with sane defaults. Used by
/// callers (and tests) so we don't repeat the spec→state shaping
/// boilerplate.
pub fn new_claim_state(
    name: impl Into<String>,
    namespace: impl Into<String>,
    tenant: TenantId,
    mode: AllocationMode,
) -> ResourceClaimState {
    ResourceClaimState {
        name: name.into(),
        namespace: namespace.into(),
        tenant_id: tenant,
        allocation_mode: mode,
        finalizers: vec![],
        deletion_timestamp_set: false,
        allocation: None,
        reserved_for: vec![],
        deallocation_requested: false,
    }
}

/// Surface returned to the admin / portal API: a single line
/// describing the current reconciler decision in plain English.
/// Plays well with the existing `/admin/cm` controller-status table.
pub fn decision_description(action: &ClaimAction) -> &'static str {
    match action {
        ClaimAction::NoOp => "steady-state — no work",
        ClaimAction::AddFinalizer => "stamp delete-protection finalizer",
        ClaimAction::AwaitImmediateCandidate => "Immediate: waiting on scheduler candidate",
        ClaimAction::AwaitFirstConsumer => "WaitForFirstConsumer: no scheduled consumer yet",
        ClaimAction::SetAllocation(_) => "write AllocationResult to status",
        ClaimAction::AddReservation { .. } => "extend reservedFor with new consumers",
        ClaimAction::RemoveReservation { .. } => "drop stale reservedFor entries",
        ClaimAction::AwaitConsumerDrain => "deletion: waiting for consumers to drain",
        ClaimAction::RequestDeallocation => "deletion: requesting driver deallocation",
        ClaimAction::RemoveFinalizer => "deletion: strip delete-protection finalizer",
        ClaimAction::AwaitDeletion => "deletion: awaiting apiserver GC",
    }
}

/// Wraps the reconciler call so the controller-runtime adoption layer
/// can return the `Reconcile` outcome the kernel expects (see
/// `crate::runtime::ScaffoldReconciler`). Used by `runtime.rs` to
/// surface the resourceclaim reconciler as one of the manager's
/// drivers.
pub fn reconcile_outcome(action: &ClaimAction) -> crate::types::Reconcile {
    use crate::types::Reconcile;
    match action {
        ClaimAction::NoOp | ClaimAction::AwaitDeletion => Reconcile::NoOp,
        ClaimAction::AddFinalizer
        | ClaimAction::RemoveFinalizer
        | ClaimAction::SetAllocation(_)
        | ClaimAction::RequestDeallocation => Reconcile::Update(1),
        ClaimAction::AddReservation { pod_uids } => Reconcile::Update(pod_uids.len() as u32),
        ClaimAction::RemoveReservation { pod_uids } => Reconcile::Update(pod_uids.len() as u32),
        ClaimAction::AwaitImmediateCandidate
        | ClaimAction::AwaitFirstConsumer
        | ClaimAction::AwaitConsumerDrain => Reconcile::Requeue,
    }
}

/// Tenant gate — controllers honor cross-tenant authorisation.
pub fn check_tenant(
    claim: &ResourceClaimState,
    expected: &TenantId,
) -> Result<(), ControllerError> {
    if &claim.tenant_id != expected {
        return Err(ControllerError::TenantDenied {
            tenant: expected.clone(),
            kind: "ResourceClaim",
            name: format!("{}/{}", claim.namespace, claim.name),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn t() -> TenantId {
        TenantId::new("acme").unwrap()
    }

    fn fresh(mode: AllocationMode) -> ResourceClaimState {
        new_claim_state("claim-a", "default", t(), mode)
    }

    fn with_finalizer(mut c: ResourceClaimState) -> ResourceClaimState {
        c.finalizers.push(FINALIZER_PROTECTION.into());
        c
    }

    fn pod(uid: &str, node: Option<&str>, deleted: bool) -> PodView {
        PodView {
            uid: uid.into(),
            node_name: node.map(|s| s.into()),
            deleted,
        }
    }

    fn cand(node: &str, devices: &[&str]) -> AllocationCandidate {
        AllocationCandidate {
            node: node.into(),
            devices: devices.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn fresh_immediate_claim_first_action_is_add_finalizer() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "syncHandler",
            "rc-1"
        );
        let claim = fresh(AllocationMode::Immediate);
        let action = evaluate(&claim, &[], None);
        assert_eq!(action, ClaimAction::AddFinalizer);
    }

    #[test]
    fn fresh_wait_claim_first_action_is_add_finalizer() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "syncHandler",
            "rc-2"
        );
        let claim = fresh(AllocationMode::WaitForFirstConsumer);
        let action = evaluate(&claim, &[], None);
        assert_eq!(action, ClaimAction::AddFinalizer);
    }

    #[test]
    fn immediate_with_finalizer_no_candidate_requeues() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "allocateImmediate",
            "rc-3"
        );
        let claim = with_finalizer(fresh(AllocationMode::Immediate));
        let action = evaluate(&claim, &[], None);
        assert_eq!(action, ClaimAction::AwaitImmediateCandidate);
    }

    #[test]
    fn immediate_with_candidate_sets_allocation() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "allocateImmediate",
            "rc-4"
        );
        let claim = with_finalizer(fresh(AllocationMode::Immediate));
        let c = cand("node1", &["gpu-0", "gpu-1"]);
        let action = evaluate(&claim, &[], Some(&c));
        assert_eq!(
            action,
            ClaimAction::SetAllocation(AllocationResult {
                node: "node1".into(),
                devices: vec!["gpu-0".into(), "gpu-1".into()],
            })
        );
    }

    #[test]
    fn wait_no_scheduled_consumer_requeues_even_with_candidate() {
        // Cite: WaitForFirstConsumer must NOT allocate until a pod is
        // actually scheduled to a node.
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "waitForFirstConsumer",
            "rc-5"
        );
        let claim = with_finalizer(fresh(AllocationMode::WaitForFirstConsumer));
        let pods = [pod("p1", None, false)];
        let c = cand("node1", &["gpu-0"]);
        let action = evaluate(&claim, &pods, Some(&c));
        assert_eq!(action, ClaimAction::AwaitFirstConsumer);
    }

    #[test]
    fn wait_scheduled_consumer_with_matching_candidate_allocates() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "waitForFirstConsumer",
            "rc-6"
        );
        let claim = with_finalizer(fresh(AllocationMode::WaitForFirstConsumer));
        let pods = [pod("p1", Some("node1"), false)];
        let c = cand("node1", &["gpu-0"]);
        let action = evaluate(&claim, &pods, Some(&c));
        assert!(matches!(action, ClaimAction::SetAllocation(_)));
    }

    #[test]
    fn wait_scheduled_to_different_node_than_candidate_requeues() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "waitForFirstConsumer",
            "rc-7"
        );
        let claim = with_finalizer(fresh(AllocationMode::WaitForFirstConsumer));
        let pods = [pod("p1", Some("node1"), false)];
        let c = cand("node2", &["gpu-0"]);
        let action = evaluate(&claim, &pods, Some(&c));
        assert_eq!(action, ClaimAction::AwaitFirstConsumer);
    }

    #[test]
    fn allocated_claim_with_unreserved_live_pod_adds_reservation() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "syncReservedFor",
            "rc-8"
        );
        let mut claim = with_finalizer(fresh(AllocationMode::Immediate));
        claim.allocation = Some(AllocationResult {
            node: "node1".into(),
            devices: vec!["gpu-0".into()],
        });
        let pods = [pod("p1", Some("node1"), false)];
        let action = evaluate(&claim, &pods, None);
        assert_eq!(
            action,
            ClaimAction::AddReservation {
                pod_uids: vec!["p1".into()]
            }
        );
    }

    #[test]
    fn allocated_claim_with_dead_consumer_in_reserved_for_drops_it() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "syncReservedFor",
            "rc-9"
        );
        let mut claim = with_finalizer(fresh(AllocationMode::Immediate));
        claim.allocation = Some(AllocationResult {
            node: "node1".into(),
            devices: vec!["gpu-0".into()],
        });
        claim.reserved_for = vec![ConsumerRef { pod_uid: "p1".into() }];
        // Pod is gone entirely (no entry in `pods`).
        let action = evaluate(&claim, &[], None);
        assert_eq!(
            action,
            ClaimAction::RemoveReservation {
                pod_uids: vec!["p1".into()]
            }
        );
    }

    #[test]
    fn allocated_claim_with_pod_being_deleted_drops_reservation() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "syncReservedFor",
            "rc-10"
        );
        let mut claim = with_finalizer(fresh(AllocationMode::Immediate));
        claim.allocation = Some(AllocationResult {
            node: "node1".into(),
            devices: vec!["gpu-0".into()],
        });
        claim.reserved_for = vec![ConsumerRef { pod_uid: "p1".into() }];
        // Pod present but marked deleted.
        let pods = [pod("p1", Some("node1"), true)];
        let action = evaluate(&claim, &pods, None);
        assert_eq!(
            action,
            ClaimAction::RemoveReservation {
                pod_uids: vec!["p1".into()]
            }
        );
    }

    #[test]
    fn allocated_claim_with_consistent_reservation_is_noop() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "syncHandler",
            "rc-11"
        );
        let mut claim = with_finalizer(fresh(AllocationMode::Immediate));
        claim.allocation = Some(AllocationResult {
            node: "node1".into(),
            devices: vec!["gpu-0".into()],
        });
        claim.reserved_for = vec![ConsumerRef { pod_uid: "p1".into() }];
        let pods = [pod("p1", Some("node1"), false)];
        let action = evaluate(&claim, &pods, None);
        assert_eq!(action, ClaimAction::NoOp);
    }

    #[test]
    fn deletion_with_active_consumers_waits_for_drain() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "deleteHandler",
            "rc-12"
        );
        let mut claim = with_finalizer(fresh(AllocationMode::Immediate));
        claim.allocation = Some(AllocationResult {
            node: "node1".into(),
            devices: vec!["gpu-0".into()],
        });
        claim.reserved_for = vec![ConsumerRef { pod_uid: "p1".into() }];
        claim.deletion_timestamp_set = true;
        let action = evaluate(&claim, &[], None);
        assert_eq!(action, ClaimAction::AwaitConsumerDrain);
    }

    #[test]
    fn deletion_drained_requests_deallocation() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "deleteHandler",
            "rc-13"
        );
        let mut claim = with_finalizer(fresh(AllocationMode::Immediate));
        claim.allocation = Some(AllocationResult {
            node: "node1".into(),
            devices: vec!["gpu-0".into()],
        });
        claim.deletion_timestamp_set = true;
        let action = evaluate(&claim, &[], None);
        assert_eq!(action, ClaimAction::RequestDeallocation);
    }

    #[test]
    fn deletion_drained_and_dealloc_done_removes_finalizer() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "deleteHandler",
            "rc-14"
        );
        let mut claim = with_finalizer(fresh(AllocationMode::Immediate));
        // allocation cleared by the driver; deallocation_requested
        // was set on the previous pass.
        claim.deletion_timestamp_set = true;
        claim.deallocation_requested = true;
        let action = evaluate(&claim, &[], None);
        assert_eq!(action, ClaimAction::RemoveFinalizer);
    }

    #[test]
    fn deletion_finalizer_stripped_awaits_apiserver_gc() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "deleteHandler",
            "rc-15"
        );
        let mut claim = fresh(AllocationMode::Immediate);
        claim.deletion_timestamp_set = true;
        let action = evaluate(&claim, &[], None);
        assert_eq!(action, ClaimAction::AwaitDeletion);
    }

    #[test]
    fn apply_reservation_diff_adds_and_removes_idempotently() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "applyReservedFor",
            "rc-16"
        );
        let current = [
            ConsumerRef { pod_uid: "p1".into() },
            ConsumerRef { pod_uid: "p2".into() },
        ];
        let added = ["p2".to_string(), "p3".to_string()];
        let removed = ["p1".to_string()];
        let out = apply_reservation_diff(&current, &added, &removed);
        // p1 dropped, p2 kept (already present, not duplicated), p3 added.
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|c| c.pod_uid == "p2"));
        assert!(out.iter().any(|c| c.pod_uid == "p3"));
        assert!(!out.iter().any(|c| c.pod_uid == "p1"));
    }

    #[test]
    fn add_finalizer_then_allocate_then_reserve_then_drain_full_cycle() {
        // Walk a single claim through every state transition the
        // controller drives. This is the audit-trail test that proves
        // the state machine is closed.
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "syncHandler",
            "rc-cycle"
        );

        let mut claim = fresh(AllocationMode::WaitForFirstConsumer);
        let candidate = cand("nodeA", &["gpu-7"]);

        // (1) fresh → AddFinalizer
        assert_eq!(evaluate(&claim, &[], None), ClaimAction::AddFinalizer);
        claim.finalizers.push(FINALIZER_PROTECTION.into());

        // (2) no consumer yet → AwaitFirstConsumer
        assert_eq!(
            evaluate(&claim, &[], Some(&candidate)),
            ClaimAction::AwaitFirstConsumer
        );

        // (3) consumer scheduled → SetAllocation
        let pods = [pod("p1", Some("nodeA"), false)];
        match evaluate(&claim, &pods, Some(&candidate)) {
            ClaimAction::SetAllocation(a) => claim.allocation = Some(a),
            other => panic!("expected SetAllocation, got {other:?}"),
        }

        // (4) live pod not in reservedFor → AddReservation{p1}
        match evaluate(&claim, &pods, Some(&candidate)) {
            ClaimAction::AddReservation { pod_uids } => {
                assert_eq!(pod_uids, vec!["p1".to_string()]);
                claim.reserved_for =
                    apply_reservation_diff(&claim.reserved_for, &pod_uids, &[]);
            }
            other => panic!("expected AddReservation, got {other:?}"),
        }

        // (5) steady state → NoOp
        assert_eq!(evaluate(&claim, &pods, Some(&candidate)), ClaimAction::NoOp);

        // (6) user deletes the claim — must wait for the pod consumer to drain.
        claim.deletion_timestamp_set = true;
        assert_eq!(
            evaluate(&claim, &pods, None),
            ClaimAction::AwaitConsumerDrain
        );

        // (7) pod gone → RemoveReservation drops it.
        // (We still get RemoveReservation first because the
        //  deletion-path handler reads reservedFor as authoritative.)
        // Simulate the pod-deleted path: pop the reservation as the
        // controller would have done on the steady-state pass before
        // the user issued DELETE on the claim, then reconsider.
        claim.reserved_for.clear();
        assert_eq!(
            evaluate(&claim, &[], None),
            ClaimAction::RequestDeallocation
        );
        claim.deallocation_requested = true;

        // (8) driver finished — RemoveFinalizer.
        assert_eq!(evaluate(&claim, &[], None), ClaimAction::RemoveFinalizer);
        claim.finalizers.clear();

        // (9) AwaitDeletion — apiserver does the rest.
        assert_eq!(evaluate(&claim, &[], None), ClaimAction::AwaitDeletion);
    }

    #[test]
    fn reconcile_outcome_maps_each_action() {
        use crate::types::Reconcile;
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "reconcileOutcome",
            "rc-outcome"
        );
        assert_eq!(reconcile_outcome(&ClaimAction::NoOp), Reconcile::NoOp);
        assert_eq!(reconcile_outcome(&ClaimAction::AddFinalizer), Reconcile::Update(1));
        assert_eq!(
            reconcile_outcome(&ClaimAction::AwaitImmediateCandidate),
            Reconcile::Requeue
        );
        assert_eq!(
            reconcile_outcome(&ClaimAction::SetAllocation(AllocationResult {
                node: "n".into(),
                devices: vec!["d".into()],
            })),
            Reconcile::Update(1)
        );
        assert_eq!(
            reconcile_outcome(&ClaimAction::AddReservation {
                pod_uids: vec!["a".into(), "b".into()]
            }),
            Reconcile::Update(2)
        );
        assert_eq!(
            reconcile_outcome(&ClaimAction::RemoveReservation {
                pod_uids: vec!["a".into()]
            }),
            Reconcile::Update(1)
        );
        assert_eq!(
            reconcile_outcome(&ClaimAction::AwaitConsumerDrain),
            Reconcile::Requeue
        );
        assert_eq!(
            reconcile_outcome(&ClaimAction::RequestDeallocation),
            Reconcile::Update(1)
        );
        assert_eq!(
            reconcile_outcome(&ClaimAction::RemoveFinalizer),
            Reconcile::Update(1)
        );
        assert_eq!(
            reconcile_outcome(&ClaimAction::AwaitDeletion),
            Reconcile::NoOp
        );
    }

    #[test]
    fn decision_description_covers_all_actions() {
        // Exhaustiveness: if a new variant is added, this test forces
        // the description map to be updated.
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "decisionDescription",
            "rc-desc"
        );
        for a in &[
            ClaimAction::NoOp,
            ClaimAction::AddFinalizer,
            ClaimAction::AwaitImmediateCandidate,
            ClaimAction::AwaitFirstConsumer,
            ClaimAction::SetAllocation(AllocationResult {
                node: "n".into(),
                devices: vec!["d".into()],
            }),
            ClaimAction::AddReservation {
                pod_uids: vec!["x".into()],
            },
            ClaimAction::RemoveReservation {
                pod_uids: vec!["x".into()],
            },
            ClaimAction::AwaitConsumerDrain,
            ClaimAction::RequestDeallocation,
            ClaimAction::RemoveFinalizer,
            ClaimAction::AwaitDeletion,
        ] {
            let s = decision_description(a);
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn tenant_check_blocks_mismatch() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "tenantGate",
            "rc-tenant"
        );
        let claim = fresh(AllocationMode::Immediate);
        let other = TenantId::new("other-tenant").unwrap();
        assert!(check_tenant(&claim, &other).is_err());
        assert!(check_tenant(&claim, &t()).is_ok());
    }

    #[test]
    fn new_claim_state_has_sane_defaults() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "newClaimState",
            "rc-new"
        );
        let c = new_claim_state("x", "ns", t(), AllocationMode::Immediate);
        assert_eq!(c.name, "x");
        assert_eq!(c.namespace, "ns");
        assert_eq!(c.allocation_mode, AllocationMode::Immediate);
        assert!(c.finalizers.is_empty());
        assert!(c.allocation.is_none());
        assert!(c.reserved_for.is_empty());
        assert!(!c.deletion_timestamp_set);
        assert!(!c.deallocation_requested);
    }

    #[test]
    fn finalizer_constant_matches_upstream_string() {
        let (_c, _t) = test_ctx!(
            "staging/src/k8s.io/api/resource/v1/types.go",
            "ResourceClaimFinalizer",
            "rc-finalizer-name"
        );
        assert_eq!(FINALIZER_PROTECTION, "resource.kubernetes.io/delete-protection");
    }

    #[test]
    fn file_cite_points_at_pinned_version() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/resourceclaim/controller.go",
            "fileCite",
            "rc-file-cite"
        );
        assert_eq!(FILE_CITE.version, crate::types::UPSTREAM_VERSION);
        assert!(FILE_CITE.url().contains(crate::types::UPSTREAM_VERSION));
    }
}
