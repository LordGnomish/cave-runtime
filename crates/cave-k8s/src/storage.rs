// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PersistentVolume + Claim + StorageClass binder facade.
//!
//! Mirrors `pkg/controller/volume/persistentvolume`.  cave-k8s is
//! CSI-only — there is no in-tree volume plugin path.  The binder
//! pairs PVCs with PVs whose access modes and capacity satisfy the
//! claim, preferring volumes whose StorageClass matches.

use serde::{Deserialize, Serialize};
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessMode {
    ReadWriteOnce,
    ReadOnlyMany,
    ReadWriteMany,
    ReadWriteOncePod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReclaimPolicy {
    Retain,
    Delete,
    /// Recycle is K8s-deprecated; cave-k8s rejects it.
    Recycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingPhase {
    Pending,
    Available,
    Bound,
    Released,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentVolume {
    pub name: String,
    pub storage_class: String,
    pub capacity_bytes: u64,
    pub access_modes: Vec<AccessMode>,
    pub reclaim_policy: ReclaimPolicy,
    pub csi_driver: String,
    pub volume_handle: String,
    pub phase: BindingPhase,
    pub claim: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentVolumeClaim {
    pub namespace: String,
    pub name: String,
    pub requested_bytes: u64,
    pub access_modes: Vec<AccessMode>,
    pub storage_class: Option<String>,
    pub bound_to: Option<String>,
    pub phase: BindingPhase,
}

pub struct Binder {
    pvs: RwLock<Vec<PersistentVolume>>,
    pvcs: RwLock<Vec<PersistentVolumeClaim>>,
}

impl Default for Binder {
    fn default() -> Self {
        Self::new()
    }
}

impl Binder {
    pub fn new() -> Self {
        Self {
            pvs: RwLock::new(Vec::new()),
            pvcs: RwLock::new(Vec::new()),
        }
    }

    pub fn add_pv(&self, pv: PersistentVolume) {
        self.pvs.write().expect("pv lock").push(pv);
    }

    pub fn add_pvc(&self, pvc: PersistentVolumeClaim) {
        self.pvcs.write().expect("pvc lock").push(pvc);
    }

    pub fn pv_count(&self) -> usize {
        self.pvs.read().expect("pv lock").len()
    }

    pub fn pvc_count(&self) -> usize {
        self.pvcs.read().expect("pvc lock").len()
    }

    /// Single binding pass.  Returns the count of newly-bound claims.
    pub fn bind_once(&self) -> usize {
        let mut pvs = self.pvs.write().expect("pv lock");
        let mut pvcs = self.pvcs.write().expect("pvc lock");
        let mut bound = 0;
        for pvc in pvcs.iter_mut() {
            if pvc.bound_to.is_some() {
                continue;
            }
            // Find an Available PV that satisfies storage class +
            // capacity + access modes.
            let pick = pvs
                .iter_mut()
                .find(|pv| {
                    pv.phase == BindingPhase::Available
                        && pv.capacity_bytes >= pvc.requested_bytes
                        && pvc
                            .access_modes
                            .iter()
                            .all(|am| pv.access_modes.contains(am))
                        && match &pvc.storage_class {
                            Some(sc) => &pv.storage_class == sc,
                            None => pv.storage_class.is_empty(),
                        }
                });
            if let Some(pv) = pick {
                pv.phase = BindingPhase::Bound;
                pv.claim = Some((pvc.namespace.clone(), pvc.name.clone()));
                pvc.bound_to = Some(pv.name.clone());
                pvc.phase = BindingPhase::Bound;
                bound += 1;
            } else {
                pvc.phase = BindingPhase::Pending;
            }
        }
        bound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pv(name: &str, sc: &str, cap: u64, modes: Vec<AccessMode>) -> PersistentVolume {
        PersistentVolume {
            name: name.into(),
            storage_class: sc.into(),
            capacity_bytes: cap,
            access_modes: modes,
            reclaim_policy: ReclaimPolicy::Delete,
            csi_driver: "csi.cave".into(),
            volume_handle: format!("vol/{name}"),
            phase: BindingPhase::Available,
            claim: None,
        }
    }

    fn pvc(name: &str, sc: &str, req: u64, modes: Vec<AccessMode>) -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            namespace: "default".into(),
            name: name.into(),
            requested_bytes: req,
            access_modes: modes,
            storage_class: Some(sc.into()),
            bound_to: None,
            phase: BindingPhase::Pending,
        }
    }

    #[test]
    fn binds_pvc_to_matching_pv() {
        let b = Binder::new();
        b.add_pv(pv("pv1", "gp3", 1024, vec![AccessMode::ReadWriteOnce]));
        b.add_pvc(pvc("c1", "gp3", 512, vec![AccessMode::ReadWriteOnce]));
        let bound = b.bind_once();
        assert_eq!(bound, 1);
    }

    #[test]
    fn pvc_too_large_stays_pending() {
        let b = Binder::new();
        b.add_pv(pv("pv1", "gp3", 1024, vec![AccessMode::ReadWriteOnce]));
        b.add_pvc(pvc("c1", "gp3", 2048, vec![AccessMode::ReadWriteOnce]));
        assert_eq!(b.bind_once(), 0);
    }

    #[test]
    fn pvc_access_mode_mismatch_no_bind() {
        let b = Binder::new();
        b.add_pv(pv("pv1", "gp3", 1024, vec![AccessMode::ReadWriteOnce]));
        b.add_pvc(pvc("c1", "gp3", 512, vec![AccessMode::ReadWriteMany]));
        assert_eq!(b.bind_once(), 0);
    }

    #[test]
    fn storage_class_must_match() {
        let b = Binder::new();
        b.add_pv(pv("pv1", "gp3", 1024, vec![AccessMode::ReadWriteOnce]));
        b.add_pvc(pvc("c1", "io2", 512, vec![AccessMode::ReadWriteOnce]));
        assert_eq!(b.bind_once(), 0);
    }

    #[test]
    fn two_pvcs_share_two_pvs() {
        let b = Binder::new();
        b.add_pv(pv("pv1", "gp3", 1024, vec![AccessMode::ReadWriteOnce]));
        b.add_pv(pv("pv2", "gp3", 1024, vec![AccessMode::ReadWriteOnce]));
        b.add_pvc(pvc("c1", "gp3", 512, vec![AccessMode::ReadWriteOnce]));
        b.add_pvc(pvc("c2", "gp3", 512, vec![AccessMode::ReadWriteOnce]));
        assert_eq!(b.bind_once(), 2);
    }

    #[test]
    fn already_bound_pv_is_skipped() {
        let b = Binder::new();
        let mut existing = pv("pv1", "gp3", 1024, vec![AccessMode::ReadWriteOnce]);
        existing.phase = BindingPhase::Bound;
        b.add_pv(existing);
        b.add_pvc(pvc("c1", "gp3", 512, vec![AccessMode::ReadWriteOnce]));
        assert_eq!(b.bind_once(), 0);
    }
}
