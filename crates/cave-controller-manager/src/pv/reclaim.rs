// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PersistentVolume reclaim — `pkg/controller/volume/persistentvolume/pv_controller.go::reclaimVolume`.
//!
//! When a bound PVC is deleted, the PV transitions to `Released`. The
//! reclaim policy on the PV decides what happens next:
//!
//! * `Retain` — PV stays in `Released`; admin must recycle/delete manually.
//! * `Delete` — controller-manager calls the dynamic provisioner to delete
//!   the underlying volume, then deletes the PV object.
//! * `Recycle` — DEPRECATED in v1.18; controller scrubs the FS and returns
//!   PV to `Available` (only NFS / HostPath plugins).

use super::binder::{PersistentVolume, PvPhase, ReclaimPolicy};
use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReclaimAction {
    /// Retain — leave PV in Released.
    NoOp,
    /// Recycle (deprecated) — scrub then return to Available.
    Recycle,
    /// Delete — call provisioner and then delete the PV object.
    DeleteVolume,
    /// Reclaim policy not applicable (PV not in Released).
    Skip,
}

pub fn evaluate(pv: &PersistentVolume) -> ReclaimAction {
    if pv.phase != PvPhase::Released {
        return ReclaimAction::Skip;
    }
    match pv.reclaim_policy {
        ReclaimPolicy::Retain => ReclaimAction::NoOp,
        ReclaimPolicy::Recycle => ReclaimAction::Recycle,
        ReclaimPolicy::Delete => ReclaimAction::DeleteVolume,
    }
}

// ── Dynamic provisioning ────────────────────────────────────────────────

/// Inputs to a dynamic-provisioning decision. Mirrors `provisionClaim`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicProvisionRequest {
    pub pvc_name: String,
    pub namespace: String,
    pub storage_class: String,
    pub allow_dynamic: bool,
    /// True when binding mode is WaitForFirstConsumer AND the PVC has not
    /// yet been bound to a node-selecting consumer.
    pub wait_for_consumer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvisionAction {
    /// Storage class is missing or doesn't allow dynamic provisioning.
    Skip(&'static str),
    /// Wait for a Pod to consume the PVC.
    WaitForConsumer,
    /// Provision now via the named provisioner.
    Provision(String),
}

pub fn evaluate_provisioning(req: &DynamicProvisionRequest) -> ProvisionAction {
    if !req.allow_dynamic {
        return ProvisionAction::Skip("storage class disallows dynamic provisioning");
    }
    if req.storage_class.is_empty() {
        return ProvisionAction::Skip("PVC has no storage class");
    }
    if req.wait_for_consumer {
        return ProvisionAction::WaitForConsumer;
    }
    ProvisionAction::Provision(req.storage_class.clone())
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/volume/persistentvolume/pv_controller.go",
    "reclaimVolume",
);

#[cfg(test)]
mod tests {
    use super::super::binder::{AccessMode, VolumeMode};
    use super::*;
    use crate::test_ctx;

    fn pv(policy: ReclaimPolicy, phase: PvPhase) -> PersistentVolume {
        PersistentVolume {
            name: "pv".into(),
            capacity_gi: 10,
            access_modes: vec![AccessMode::Rwo],
            volume_mode: VolumeMode::Filesystem,
            storage_class: "fast".into(),
            phase,
            reclaim_policy: policy,
        }
    }

    #[test]
    fn retain_in_released_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "reclaimVolume",
            "tenant-pv-rec-retain"
        );
        assert_eq!(
            evaluate(&pv(ReclaimPolicy::Retain, PvPhase::Released)),
            ReclaimAction::NoOp
        );
    }

    #[test]
    fn delete_in_released_triggers_volume_deletion() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "reclaimVolume",
            "tenant-pv-rec-delete"
        );
        assert_eq!(
            evaluate(&pv(ReclaimPolicy::Delete, PvPhase::Released)),
            ReclaimAction::DeleteVolume
        );
    }

    #[test]
    fn recycle_emits_recycle_action() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "recycleVolumeOperation",
            "tenant-pv-rec-recycle"
        );
        assert_eq!(
            evaluate(&pv(ReclaimPolicy::Recycle, PvPhase::Released)),
            ReclaimAction::Recycle
        );
    }

    #[test]
    fn non_released_phase_is_skipped() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "reclaimVolume",
            "tenant-pv-rec-skip"
        );
        for phase in [
            PvPhase::Available,
            PvPhase::Bound,
            PvPhase::Pending,
            PvPhase::Failed,
        ] {
            assert_eq!(
                evaluate(&pv(ReclaimPolicy::Delete, phase)),
                ReclaimAction::Skip
            );
        }
    }

    #[test]
    fn provision_skipped_when_allow_dynamic_false() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "provisionClaim",
            "tenant-pv-prov-not-allowed"
        );
        let req = DynamicProvisionRequest {
            pvc_name: "p".into(),
            namespace: "default".into(),
            storage_class: "standard".into(),
            allow_dynamic: false,
            wait_for_consumer: false,
        };
        match evaluate_provisioning(&req) {
            ProvisionAction::Skip(_) => {}
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn provision_skipped_when_no_storage_class() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "provisionClaim",
            "tenant-pv-prov-no-sc"
        );
        let req = DynamicProvisionRequest {
            pvc_name: "p".into(),
            namespace: "default".into(),
            storage_class: String::new(),
            allow_dynamic: true,
            wait_for_consumer: false,
        };
        match evaluate_provisioning(&req) {
            ProvisionAction::Skip(_) => {}
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn provision_waits_for_consumer_under_wffc() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "provisionClaim",
            "tenant-pv-prov-wffc"
        );
        let req = DynamicProvisionRequest {
            pvc_name: "p".into(),
            namespace: "default".into(),
            storage_class: "fast".into(),
            allow_dynamic: true,
            wait_for_consumer: true,
        };
        assert_eq!(
            evaluate_provisioning(&req),
            ProvisionAction::WaitForConsumer
        );
    }

    #[test]
    fn provision_emits_provisioner_name() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "provisionClaim",
            "tenant-pv-prov-now"
        );
        let req = DynamicProvisionRequest {
            pvc_name: "p".into(),
            namespace: "default".into(),
            storage_class: "fast".into(),
            allow_dynamic: true,
            wait_for_consumer: false,
        };
        assert_eq!(
            evaluate_provisioning(&req),
            ProvisionAction::Provision("fast".into())
        );
    }

    #[test]
    fn reclaim_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "ReclaimAction",
            "tenant-pv-rec-serde"
        );
        for a in [
            ReclaimAction::NoOp,
            ReclaimAction::Recycle,
            ReclaimAction::DeleteVolume,
            ReclaimAction::Skip,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: ReclaimAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}
