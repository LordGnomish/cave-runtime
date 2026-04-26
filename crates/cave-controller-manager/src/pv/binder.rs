//! PV/PVC binder — `pkg/controller/volume/persistentvolume/pv_controller.go`.
//!
//! Reconciles PersistentVolumeClaims with PersistentVolumes:
//!
//! * **Immediate binding** (default `volumeBindingMode`): on PVC creation,
//!   pick the smallest PV that satisfies access modes, capacity, storage
//!   class, volume mode → bind.
//! * **WaitForFirstConsumer**: bind only after a Pod referencing the PVC is
//!   scheduled — defers PV selection so the scheduler can pick zone-aware.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessMode {
    /// `ReadWriteOnce` — single-node mount.
    Rwo,
    /// `ReadOnlyMany` — multi-node read.
    Rox,
    /// `ReadWriteMany` — multi-node read/write.
    Rwx,
    /// `ReadWriteOncePod` — single-pod mount.
    Rwop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeMode {
    Filesystem,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingMode {
    Immediate,
    WaitForFirstConsumer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReclaimPolicy {
    Retain,
    Delete,
    Recycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PvPhase {
    Available,
    Bound,
    Released,
    Failed,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PvcPhase {
    Pending,
    Bound,
    Lost,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentVolume {
    pub name: String,
    pub capacity_gi: u64,
    pub access_modes: Vec<AccessMode>,
    pub volume_mode: VolumeMode,
    pub storage_class: String,
    pub phase: PvPhase,
    pub reclaim_policy: ReclaimPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentVolumeClaim {
    pub name: String,
    pub namespace: String,
    pub request_gi: u64,
    pub access_modes: Vec<AccessMode>,
    pub volume_mode: VolumeMode,
    pub storage_class: String,
    pub binding_mode: BindingMode,
    pub phase: PvcPhase,
    /// Set to true when at least one Pod referencing this claim has been
    /// scheduled — relevant only for WaitForFirstConsumer.
    pub has_scheduled_consumer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindAction {
    /// PVC remains pending — no acceptable PV (or WaitForFirstConsumer with
    /// no scheduled consumer yet).
    Wait,
    /// Bind PVC to the named PV.
    Bind(String),
    /// PVC already bound — no work.
    NoOp,
}

/// Returns true if `pv` satisfies all of `pvc`'s requirements.
pub fn pv_satisfies(pv: &PersistentVolume, pvc: &PersistentVolumeClaim) -> bool {
    if pv.phase != PvPhase::Available {
        return false;
    }
    if pv.storage_class != pvc.storage_class {
        return false;
    }
    if pv.volume_mode != pvc.volume_mode {
        return false;
    }
    if pv.capacity_gi < pvc.request_gi {
        return false;
    }
    // PV must support every access mode the PVC asks for.
    for req in &pvc.access_modes {
        if !pv.access_modes.contains(req) {
            return false;
        }
    }
    true
}

/// Pick the best matching PV for a PVC: smallest-fit (least waste).
/// Mirrors `findBestMatchForClaim` in upstream's volume scheduler helper.
pub fn pick_pv<'a>(
    pvc: &PersistentVolumeClaim,
    pvs: &'a [PersistentVolume],
) -> Option<&'a PersistentVolume> {
    pvs.iter()
        .filter(|pv| pv_satisfies(pv, pvc))
        .min_by_key(|pv| pv.capacity_gi)
}

/// Decide what to do with the PVC under the binder controller.
pub fn evaluate(
    pvc: &PersistentVolumeClaim,
    pvs: &[PersistentVolume],
) -> BindAction {
    if pvc.phase == PvcPhase::Bound {
        return BindAction::NoOp;
    }
    if pvc.binding_mode == BindingMode::WaitForFirstConsumer && !pvc.has_scheduled_consumer {
        return BindAction::Wait;
    }
    match pick_pv(pvc, pvs) {
        Some(pv) => BindAction::Bind(pv.name.clone()),
        None => BindAction::Wait,
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/volume/persistentvolume/pv_controller.go",
    "PersistentVolumeController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn pv(name: &str, gi: u64, modes: Vec<AccessMode>, sc: &str, phase: PvPhase) -> PersistentVolume {
        PersistentVolume {
            name: name.into(),
            capacity_gi: gi,
            access_modes: modes,
            volume_mode: VolumeMode::Filesystem,
            storage_class: sc.into(),
            phase,
            reclaim_policy: ReclaimPolicy::Delete,
        }
    }

    fn pvc(
        gi: u64,
        modes: Vec<AccessMode>,
        sc: &str,
        bm: BindingMode,
        phase: PvcPhase,
    ) -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            name: "pvc".into(),
            namespace: "default".into(),
            request_gi: gi,
            access_modes: modes,
            volume_mode: VolumeMode::Filesystem,
            storage_class: sc.into(),
            binding_mode: bm,
            phase,
            has_scheduled_consumer: false,
        }
    }

    #[test]
    fn satisfies_when_all_attrs_match() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "checkVolumeSatisfyClaim",
            "tenant-pv-satisfies"
        );
        let p = pv("pv1", 10, vec![AccessMode::Rwo], "fast", PvPhase::Available);
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert!(pv_satisfies(&p, &c));
    }

    #[test]
    fn rejects_capacity_too_small() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "checkVolumeSatisfyClaim",
            "tenant-pv-cap-small"
        );
        let p = pv("pv1", 5, vec![AccessMode::Rwo], "fast", PvPhase::Available);
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert!(!pv_satisfies(&p, &c));
    }

    #[test]
    fn rejects_storage_class_mismatch() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "checkVolumeSatisfyClaim",
            "tenant-pv-sc-mismatch"
        );
        let p = pv("pv1", 10, vec![AccessMode::Rwo], "slow", PvPhase::Available);
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert!(!pv_satisfies(&p, &c));
    }

    #[test]
    fn rejects_missing_access_mode() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "checkVolumeSatisfyClaim",
            "tenant-pv-mode-missing"
        );
        let p = pv("pv1", 10, vec![AccessMode::Rwo], "fast", PvPhase::Available);
        let c = pvc(10, vec![AccessMode::Rwx], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert!(!pv_satisfies(&p, &c));
    }

    #[test]
    fn rejects_volume_mode_mismatch() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "checkVolumeSatisfyClaim",
            "tenant-pv-vm-mismatch"
        );
        let mut p = pv("pv1", 10, vec![AccessMode::Rwo], "fast", PvPhase::Available);
        p.volume_mode = VolumeMode::Block;
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert!(!pv_satisfies(&p, &c));
    }

    #[test]
    fn rejects_already_bound_pv() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "checkVolumeSatisfyClaim",
            "tenant-pv-bound"
        );
        let p = pv("pv1", 10, vec![AccessMode::Rwo], "fast", PvPhase::Bound);
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert!(!pv_satisfies(&p, &c));
    }

    #[test]
    fn pick_pv_returns_smallest_fit() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "findBestMatchForClaim",
            "tenant-pv-smallest-fit"
        );
        let pvs = vec![
            pv("big", 100, vec![AccessMode::Rwo], "fast", PvPhase::Available),
            pv("medium", 20, vec![AccessMode::Rwo], "fast", PvPhase::Available),
            pv("just-right", 11, vec![AccessMode::Rwo], "fast", PvPhase::Available),
        ];
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert_eq!(pick_pv(&c, &pvs).unwrap().name, "just-right");
    }

    #[test]
    fn pick_pv_returns_none_when_no_match() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "findBestMatchForClaim",
            "tenant-pv-no-match"
        );
        let pvs = vec![pv("p", 5, vec![AccessMode::Rwo], "slow", PvPhase::Available)];
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert!(pick_pv(&c, &pvs).is_none());
    }

    #[test]
    fn evaluate_immediate_binds_when_match_present() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "syncClaim",
            "tenant-pv-eval-bind"
        );
        let pvs = vec![pv("pv1", 10, vec![AccessMode::Rwo], "fast", PvPhase::Available)];
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert_eq!(evaluate(&c, &pvs), BindAction::Bind("pv1".into()));
    }

    #[test]
    fn evaluate_immediate_waits_when_no_match() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "syncClaim",
            "tenant-pv-eval-wait"
        );
        let pvs: Vec<_> = vec![];
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert_eq!(evaluate(&c, &pvs), BindAction::Wait);
    }

    #[test]
    fn evaluate_wait_for_first_consumer_waits_without_scheduled_pod() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "syncClaim",
            "tenant-pv-eval-wffc-wait"
        );
        let pvs = vec![pv("pv1", 10, vec![AccessMode::Rwo], "fast", PvPhase::Available)];
        let c = pvc(
            10,
            vec![AccessMode::Rwo],
            "fast",
            BindingMode::WaitForFirstConsumer,
            PvcPhase::Pending,
        );
        // No consumer yet → wait even though a PV is available.
        assert_eq!(evaluate(&c, &pvs), BindAction::Wait);
    }

    #[test]
    fn evaluate_wait_for_first_consumer_binds_after_scheduling() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "syncClaim",
            "tenant-pv-eval-wffc-bind"
        );
        let pvs = vec![pv("pv1", 10, vec![AccessMode::Rwo], "fast", PvPhase::Available)];
        let mut c = pvc(
            10,
            vec![AccessMode::Rwo],
            "fast",
            BindingMode::WaitForFirstConsumer,
            PvcPhase::Pending,
        );
        c.has_scheduled_consumer = true;
        assert_eq!(evaluate(&c, &pvs), BindAction::Bind("pv1".into()));
    }

    #[test]
    fn evaluate_already_bound_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "syncClaim",
            "tenant-pv-eval-already-bound"
        );
        let pvs = vec![pv("pv1", 10, vec![AccessMode::Rwo], "fast", PvPhase::Available)];
        let c = pvc(10, vec![AccessMode::Rwo], "fast", BindingMode::Immediate, PvcPhase::Bound);
        assert_eq!(evaluate(&c, &pvs), BindAction::NoOp);
    }

    #[test]
    fn rwx_pvc_selects_only_rwx_pv() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "checkVolumeSatisfyClaim",
            "tenant-pv-rwx"
        );
        let pvs = vec![
            pv("rwo", 10, vec![AccessMode::Rwo], "fast", PvPhase::Available),
            pv("rwx", 10, vec![AccessMode::Rwx], "fast", PvPhase::Available),
        ];
        let c = pvc(10, vec![AccessMode::Rwx], "fast", BindingMode::Immediate, PvcPhase::Pending);
        assert_eq!(pick_pv(&c, &pvs).unwrap().name, "rwx");
    }

    #[test]
    fn bind_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "BindAction",
            "tenant-pv-action-serde"
        );
        for a in [
            BindAction::Wait,
            BindAction::Bind("pv1".into()),
            BindAction::NoOp,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: BindAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }

    #[test]
    fn multi_access_mode_pvc_requires_pv_to_support_all() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/volume/persistentvolume/pv_controller.go",
            "checkVolumeSatisfyClaim",
            "tenant-pv-multi-mode"
        );
        let p = pv(
            "p",
            10,
            vec![AccessMode::Rwo, AccessMode::Rox],
            "fast",
            PvPhase::Available,
        );
        let c = pvc(
            10,
            vec![AccessMode::Rwo, AccessMode::Rox],
            "fast",
            BindingMode::Immediate,
            PvcPhase::Pending,
        );
        assert!(pv_satisfies(&p, &c));
        // Drop one mode from the PV — no longer satisfies.
        let p2 = pv("p", 10, vec![AccessMode::Rwo], "fast", PvPhase::Available);
        assert!(!pv_satisfies(&p2, &c));
    }
}
