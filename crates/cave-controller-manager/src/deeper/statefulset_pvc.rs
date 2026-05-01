//! StatefulSet — ordered creation + PVC binding state machine + scale-down
//! + set-deletion cascade.
//!
//! Mirrors `pkg/controller/statefulset/stateful_set_control.go` plus the
//! PVC retention plumbing in `pkg/controller/statefulset/stateful_pod_control.go`.
//! The `StatefulSetWorld` struct is an in-memory ledger of the (pod, pvc)
//! pairs the controller has already created so the test harness can drive
//! ordinal reconciliation without a real apiserver.

use crate::types::{Cite, ControllerError, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PvcRetentionPolicy {
    /// Keep PVCs forever (the v1.36 default).
    Retain,
    /// Delete the PVC when the owning pod is removed.
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PvcPhase {
    Pending,
    Bound { pv_id: String },
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodPhase {
    Pending,
    Running,
    Terminating,
    Gone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodSlot {
    pub ordinal: u32,
    pub pod_phase: PodPhase,
    pub pvc_phase: PvcPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatefulSetWorld {
    pub name: String,
    pub tenant: TenantId,
    pub desired_replicas: u32,
    pub retention: PvcRetentionPolicy,
    pub slots: Vec<PodSlot>,
    /// Deletion timestamp set → `cascade_delete_set` is allowed.
    pub deletion_pending: bool,
}

/// One reconciliation step. Mirrors `updateStatefulSetReplicas` in upstream;
/// the `OrderedReady` policy is enforced (only one slot mutates per pass).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    /// No change required.
    NoOp,
    /// Create a new pod (and its PVC) at this ordinal.
    CreateOrdinal(u32),
    /// Wait for an in-flight bind to complete.
    AwaitBind(u32),
    /// Begin termination of the highest-ordinal pod.
    BeginTerminate(u32),
    /// Pod has gone; reclaim its PVC if retention policy allows.
    ReclaimPvc(u32),
    /// Set is being deleted — tear down highest ordinal first.
    Cascade(u32),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StsError {
    #[error("ordinals in slot list are not contiguous starting at 0")]
    NonContiguous,
    #[error("tenant {tenant} cannot drive set owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

impl StatefulSetWorld {
    pub fn new(tenant: TenantId, name: impl Into<String>, desired: u32) -> Self {
        Self {
            name: name.into(),
            tenant,
            desired_replicas: desired,
            retention: PvcRetentionPolicy::Retain,
            slots: Vec::new(),
            deletion_pending: false,
        }
    }

    pub fn with_retention(mut self, p: PvcRetentionPolicy) -> Self {
        self.retention = p;
        self
    }

    fn assert_contiguous(&self) -> Result<(), StsError> {
        for (i, s) in self.slots.iter().enumerate() {
            if s.ordinal != i as u32 {
                return Err(StsError::NonContiguous);
            }
        }
        Ok(())
    }

    /// Decide one mutation step. Mirrors the per-pass body of
    /// `updateStatefulSet` in upstream.
    pub fn next_step(&self, caller: &TenantId) -> Result<Step, ControllerError> {
        if caller != &self.tenant {
            return Err(ControllerError::TenantDenied {
                tenant: caller.clone(),
                kind: "StatefulSet",
                name: self.name.clone(),
            });
        }
        self.assert_contiguous().map_err(|e| ControllerError::InvalidSpec {
            kind: "StatefulSet",
            reason: e.to_string(),
        })?;
        if self.deletion_pending {
            // Cascade: highest ordinal that still has a pod.
            for s in self.slots.iter().rev() {
                if s.pod_phase != PodPhase::Gone {
                    return Ok(Step::Cascade(s.ordinal));
                }
            }
            return Ok(Step::NoOp);
        }
        let live = self.slots.iter().filter(|s| s.pod_phase != PodPhase::Gone).count() as u32;
        // Scale up: create the next ordinal.
        if live < self.desired_replicas {
            // Find first ordinal that is missing or Gone.
            for ord in 0..self.desired_replicas {
                let slot = self.slots.iter().find(|s| s.ordinal == ord);
                match slot {
                    None | Some(PodSlot { pod_phase: PodPhase::Gone, .. }) => {
                        return Ok(Step::CreateOrdinal(ord));
                    }
                    Some(s) if !matches!(s.pvc_phase, PvcPhase::Bound { .. }) => {
                        // The slot exists but its PVC hasn't bound yet — must
                        // wait before touching the next ordinal.
                        return Ok(Step::AwaitBind(s.ordinal));
                    }
                    _ => continue,
                }
            }
        }
        // Scale down: terminate the highest live ordinal.
        if live > self.desired_replicas {
            let candidate = self
                .slots
                .iter()
                .rev()
                .find(|s| s.pod_phase == PodPhase::Running)
                .map(|s| s.ordinal);
            if let Some(o) = candidate {
                return Ok(Step::BeginTerminate(o));
            }
        }
        // Reclaim phase: any Gone pod whose PVC still exists and policy is Delete.
        if self.retention == PvcRetentionPolicy::Delete {
            for s in &self.slots {
                if s.pod_phase == PodPhase::Gone && s.pvc_phase != PvcPhase::Lost {
                    return Ok(Step::ReclaimPvc(s.ordinal));
                }
            }
        }
        Ok(Step::NoOp)
    }

    /// Apply a step (mutates state). Used by the test harness to drive the
    /// state machine forward.
    pub fn apply(&mut self, step: &Step) {
        match *step {
            Step::CreateOrdinal(o) => {
                if let Some(s) = self.slots.iter_mut().find(|s| s.ordinal == o) {
                    s.pod_phase = PodPhase::Pending;
                    s.pvc_phase = PvcPhase::Pending;
                } else {
                    self.slots.push(PodSlot {
                        ordinal: o,
                        pod_phase: PodPhase::Pending,
                        pvc_phase: PvcPhase::Pending,
                    });
                    self.slots.sort_by_key(|s| s.ordinal);
                }
            }
            Step::AwaitBind(_) => {}
            Step::BeginTerminate(o) => {
                if let Some(s) = self.slots.iter_mut().find(|s| s.ordinal == o) {
                    s.pod_phase = PodPhase::Terminating;
                }
            }
            Step::ReclaimPvc(o) => {
                if let Some(s) = self.slots.iter_mut().find(|s| s.ordinal == o) {
                    s.pvc_phase = PvcPhase::Lost;
                }
            }
            Step::Cascade(o) => {
                if let Some(s) = self.slots.iter_mut().find(|s| s.ordinal == o) {
                    s.pod_phase = PodPhase::Gone;
                    if self.retention == PvcRetentionPolicy::Delete {
                        s.pvc_phase = PvcPhase::Lost;
                    }
                }
            }
            Step::NoOp => {}
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/statefulset/stateful_set_control.go",
    "updateStatefulSetReplicas",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn world(desired: u32) -> StatefulSetWorld {
        StatefulSetWorld::new(TenantId::new("acme").expect("test fixture"), "db", desired)
    }

    fn bound(o: u32) -> PodSlot {
        PodSlot { ordinal: o, pod_phase: PodPhase::Running, pvc_phase: PvcPhase::Bound { pv_id: format!("pv-{o}") } }
    }

    #[test]
    fn empty_set_creates_ordinal_zero_first() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "createPodIfMissing",
            "acme"
        );
        let w = world(3);
        assert_eq!(w.next_step(&tenant).unwrap(), Step::CreateOrdinal(0));
    }

    #[test]
    fn next_step_blocks_until_pvc_bound() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_pod_control.go",
            "WaitForPersistentVolumeClaims",
            "acme"
        );
        let mut w = world(3);
        // Pod 0 created but PVC still Pending.
        w.slots.push(PodSlot {
            ordinal: 0,
            pod_phase: PodPhase::Pending,
            pvc_phase: PvcPhase::Pending,
        });
        assert_eq!(w.next_step(&tenant).unwrap(), Step::AwaitBind(0));
    }

    #[test]
    fn next_step_creates_next_ordinal_after_predecessor_bound() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "createPodIfMissing",
            "acme"
        );
        let mut w = world(3);
        w.slots.push(bound(0));
        assert_eq!(w.next_step(&tenant).unwrap(), Step::CreateOrdinal(1));
    }

    #[test]
    fn scale_down_terminates_highest_ordinal_first() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "deleteStatefulPodAtOrdinal",
            "acme"
        );
        let mut w = world(2);
        w.slots = vec![bound(0), bound(1), bound(2)];
        assert_eq!(w.next_step(&tenant).unwrap(), Step::BeginTerminate(2));
    }

    #[test]
    fn retention_delete_reclaims_pvc_after_pod_gone() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_pod_control.go",
            "DeletePersistentVolumeClaims",
            "acme"
        );
        let mut w = world(1).with_retention(PvcRetentionPolicy::Delete);
        w.slots = vec![
            bound(0),
            PodSlot { ordinal: 1, pod_phase: PodPhase::Gone, pvc_phase: PvcPhase::Bound { pv_id: "pv-1".into() } },
        ];
        // Set has one live (ord 0) which equals desired; PVC at ord 1 must be reclaimed.
        assert_eq!(w.next_step(&tenant).unwrap(), Step::ReclaimPvc(1));
    }

    #[test]
    fn retain_policy_skips_pvc_reclaim() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_pod_control.go",
            "DeletePersistentVolumeClaims",
            "acme"
        );
        let mut w = world(1); // default Retain
        w.slots = vec![
            bound(0),
            PodSlot { ordinal: 1, pod_phase: PodPhase::Gone, pvc_phase: PvcPhase::Bound { pv_id: "pv-1".into() } },
        ];
        assert_eq!(w.next_step(&tenant).unwrap(), Step::NoOp);
    }

    #[test]
    fn cascade_deletes_highest_ordinal_first_when_set_is_being_deleted() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "DeleteStatefulSet",
            "acme"
        );
        let mut w = world(3);
        w.slots = vec![bound(0), bound(1), bound(2)];
        w.deletion_pending = true;
        assert_eq!(w.next_step(&tenant).unwrap(), Step::Cascade(2));
        w.apply(&Step::Cascade(2));
        assert_eq!(w.next_step(&tenant).unwrap(), Step::Cascade(1));
    }

    #[test]
    fn next_step_refuses_cross_tenant_caller() {
        let (_cite, attacker) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "tenantCheck",
            "tenant-attacker"
        );
        let w = world(3);
        let err = w.next_step(&attacker).unwrap_err();
        assert!(matches!(err, ControllerError::TenantDenied { .. }));
    }

    #[test]
    fn non_contiguous_ordinals_are_invalid_spec() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "validateOrdinals",
            "acme"
        );
        let mut w = world(3);
        w.slots = vec![bound(0), bound(2)];
        let err = w.next_step(&tenant).unwrap_err();
        assert!(matches!(err, ControllerError::InvalidSpec { .. }));
    }

    #[test]
    fn apply_create_then_advance_drives_full_scale_up() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "updateStatefulSetReplicas",
            "acme"
        );
        let mut w = world(2);
        // Pass 1: create ord 0
        let s = w.next_step(&tenant).unwrap();
        assert_eq!(s, Step::CreateOrdinal(0));
        w.apply(&s);
        // Pass 2: PVC still Pending → AwaitBind
        assert_eq!(w.next_step(&tenant).unwrap(), Step::AwaitBind(0));
        // Simulate bind
        w.slots[0].pvc_phase = PvcPhase::Bound { pv_id: "pv-0".into() };
        w.slots[0].pod_phase = PodPhase::Running;
        // Pass 3: create ord 1
        assert_eq!(w.next_step(&tenant).unwrap(), Step::CreateOrdinal(1));
    }
}
