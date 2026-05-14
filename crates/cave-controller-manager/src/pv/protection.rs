// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PV / PVC protection finalizers — `pkg/controller/volume/pvprotection`
//! and `pkg/controller/volume/pvcprotection`.
//!
//! Two simple finalizer-based controllers:
//!
//! * **PVC protection** stamps `kubernetes.io/pvc-protection` so the PVC
//!   cannot be deleted while a Pod still references it. The finalizer is
//!   removed once `pods_using == 0`.
//! * **PV protection** stamps `kubernetes.io/pv-protection` so the PV
//!   cannot be deleted while bound to a PVC.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

pub const FINALIZER_PVC_PROTECTION: &str = "kubernetes.io/pvc-protection";
pub const FINALIZER_PV_PROTECTION: &str = "kubernetes.io/pv-protection";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinalizerOp {
    Add,
    Remove,
    NoOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PvcProtectionView {
    pub deletion_timestamp_set: bool,
    pub finalizers: Vec<String>,
    pub pods_using: u32,
}

pub fn evaluate_pvc(view: &PvcProtectionView) -> FinalizerOp {
    let has_fin = view.finalizers.iter().any(|f| f == FINALIZER_PVC_PROTECTION);
    if view.deletion_timestamp_set {
        // Only remove once nothing references the PVC.
        if has_fin && view.pods_using == 0 {
            return FinalizerOp::Remove;
        }
        return FinalizerOp::NoOp;
    }
    // Live PVC: ensure the finalizer is present.
    if has_fin { FinalizerOp::NoOp } else { FinalizerOp::Add }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PvProtectionView {
    pub deletion_timestamp_set: bool,
    pub finalizers: Vec<String>,
    pub bound_to_pvc: bool,
}

pub fn evaluate_pv(view: &PvProtectionView) -> FinalizerOp {
    let has_fin = view.finalizers.iter().any(|f| f == FINALIZER_PV_PROTECTION);
    if view.deletion_timestamp_set {
        if has_fin && !view.bound_to_pvc {
            return FinalizerOp::Remove;
        }
        return FinalizerOp::NoOp;
    }
    if has_fin { FinalizerOp::NoOp } else { FinalizerOp::Add }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/volume/pvcprotection/pvc_protection_controller.go",
    "Controller",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn pvc_view(dt: bool, fins: &[&str], pods: u32) -> PvcProtectionView {
        PvcProtectionView {
            deletion_timestamp_set: dt,
            finalizers: fins.iter().map(|s| s.to_string()).collect(),
            pods_using: pods,
        }
    }
    fn pv_view(dt: bool, fins: &[&str], bound: bool) -> PvProtectionView {
        PvProtectionView {
            deletion_timestamp_set: dt,
            finalizers: fins.iter().map(|s| s.to_string()).collect(),
            bound_to_pvc: bound,
        }
    }

    #[test]
    fn live_pvc_without_finalizer_gets_one() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvcprotection/pvc_protection_controller.go",
            "syncPVC",
            "tenant-pvc-prot-add"
        );
        assert_eq!(evaluate_pvc(&pvc_view(false, &[], 0)), FinalizerOp::Add);
    }

    #[test]
    fn live_pvc_with_finalizer_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvcprotection/pvc_protection_controller.go",
            "syncPVC",
            "tenant-pvc-prot-noop"
        );
        assert_eq!(
            evaluate_pvc(&pvc_view(false, &[FINALIZER_PVC_PROTECTION], 0)),
            FinalizerOp::NoOp
        );
    }

    #[test]
    fn deleting_pvc_with_pods_using_blocks_finalizer_removal() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvcprotection/pvc_protection_controller.go",
            "syncPVC",
            "tenant-pvc-prot-block"
        );
        assert_eq!(
            evaluate_pvc(&pvc_view(true, &[FINALIZER_PVC_PROTECTION], 1)),
            FinalizerOp::NoOp
        );
    }

    #[test]
    fn deleting_pvc_with_no_pods_removes_finalizer() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvcprotection/pvc_protection_controller.go",
            "syncPVC",
            "tenant-pvc-prot-remove"
        );
        assert_eq!(
            evaluate_pvc(&pvc_view(true, &[FINALIZER_PVC_PROTECTION], 0)),
            FinalizerOp::Remove
        );
    }

    #[test]
    fn deleting_pvc_already_clean_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvcprotection/pvc_protection_controller.go",
            "syncPVC",
            "tenant-pvc-prot-already-clean"
        );
        assert_eq!(evaluate_pvc(&pvc_view(true, &[], 0)), FinalizerOp::NoOp);
    }

    #[test]
    fn live_pv_without_finalizer_gets_one() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvprotection/pv_protection_controller.go",
            "syncPV",
            "tenant-pv-prot-add"
        );
        assert_eq!(evaluate_pv(&pv_view(false, &[], false)), FinalizerOp::Add);
    }

    #[test]
    fn deleting_pv_bound_to_pvc_blocks_removal() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvprotection/pv_protection_controller.go",
            "syncPV",
            "tenant-pv-prot-bound-block"
        );
        assert_eq!(
            evaluate_pv(&pv_view(true, &[FINALIZER_PV_PROTECTION], true)),
            FinalizerOp::NoOp
        );
    }

    #[test]
    fn deleting_pv_unbound_removes_finalizer() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvprotection/pv_protection_controller.go",
            "syncPV",
            "tenant-pv-prot-unbound-remove"
        );
        assert_eq!(
            evaluate_pv(&pv_view(true, &[FINALIZER_PV_PROTECTION], false)),
            FinalizerOp::Remove
        );
    }

    #[test]
    fn finalizer_constants_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvprotection/pv_protection_controller.go",
            "PVProtectionFinalizer",
            "tenant-prot-const"
        );
        assert_eq!(FINALIZER_PVC_PROTECTION, "kubernetes.io/pvc-protection");
        assert_eq!(FINALIZER_PV_PROTECTION, "kubernetes.io/pv-protection");
    }

    #[test]
    fn finalizer_op_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/pvprotection/pv_protection_controller.go",
            "FinalizerOp",
            "tenant-prot-serde"
        );
        for op in [FinalizerOp::Add, FinalizerOp::Remove, FinalizerOp::NoOp] {
            let s = serde_json::to_string(&op).unwrap();
            let back: FinalizerOp = serde_json::from_str(&s).unwrap();
            assert_eq!(op, back);
        }
    }
}
