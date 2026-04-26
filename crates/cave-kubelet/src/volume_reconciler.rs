//! Volume reconciler — DSW (Desired State of World) vs ASW (Actual State
//! of World) reconcile loop.
//!
//! Mirrors Kubernetes v1.36.0 upstream:
//!   `pkg/kubelet/volumemanager/cache/desired_state_of_world.go`
//!   `pkg/kubelet/volumemanager/cache/actual_state_of_world.go`
//!   `pkg/kubelet/volumemanager/reconciler/reconciler.go`
//!     (`reconcile`, `mountAttachVolumes`, `unmountVolumes`).
//!
//! Pattern:
//!
//!   * DSW lists the (pod, volume) pairs the kubelet *wants* mounted.
//!   * ASW lists the (pod, volume) pairs that are *currently* mounted.
//!   * `reconcile()` returns two action lists: `to_mount` (in DSW but not
//!     ASW) and `to_unmount` (in ASW but not DSW).  Operations are
//!     idempotent and tenant-scoped.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VolumeError {
    #[error("volume '{0}' has no mount record")]
    NotMounted(String),
    #[error("volume '{0}' already attached")]
    AlreadyAttached(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VolumeKey {
    pub pod_uid: Uuid,
    pub volume_name: String,
    pub tenant_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeSpec {
    pub key: VolumeKey,
    pub plugin: String,
    pub fs_type: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountRecord {
    pub key: VolumeKey,
    pub device_path: String,
    pub mount_path: String,
    pub mounted_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct DesiredStateOfWorld {
    /// Wanted mounts.
    pub specs: BTreeMap<VolumeKey, VolumeSpec>,
}

impl DesiredStateOfWorld {
    pub fn add_pod_volume(&mut self, spec: VolumeSpec) {
        self.specs.insert(spec.key.clone(), spec);
    }
    pub fn remove_pod_volume(&mut self, key: &VolumeKey) {
        self.specs.remove(key);
    }
    pub fn keys(&self) -> BTreeSet<VolumeKey> {
        self.specs.keys().cloned().collect()
    }
}

#[derive(Debug, Default)]
pub struct ActualStateOfWorld {
    pub mounted: BTreeMap<VolumeKey, MountRecord>,
    pub attached_devices: BTreeSet<String>,
}

impl ActualStateOfWorld {
    pub fn record_attach(&mut self, device_path: &str) -> Result<(), VolumeError> {
        if !self.attached_devices.insert(device_path.into()) {
            return Err(VolumeError::AlreadyAttached(device_path.into()));
        }
        Ok(())
    }
    pub fn record_mount(&mut self, rec: MountRecord) {
        self.mounted.insert(rec.key.clone(), rec);
    }
    pub fn record_unmount(&mut self, key: &VolumeKey) -> Result<MountRecord, VolumeError> {
        self.mounted
            .remove(key)
            .ok_or_else(|| VolumeError::NotMounted(key.volume_name.clone()))
    }
    pub fn record_detach(&mut self, device_path: &str) {
        self.attached_devices.remove(device_path);
    }
    pub fn keys(&self) -> BTreeSet<VolumeKey> {
        self.mounted.keys().cloned().collect()
    }
}

#[derive(Debug)]
pub struct ReconcilePlan {
    pub to_mount: Vec<VolumeSpec>,
    pub to_unmount: Vec<VolumeKey>,
}

/// Compute the diff between DSW and ASW.
pub fn reconcile(dsw: &DesiredStateOfWorld, asw: &ActualStateOfWorld) -> ReconcilePlan {
    let want = dsw.keys();
    let have = asw.keys();
    let to_mount: Vec<VolumeSpec> = want
        .difference(&have)
        .filter_map(|k| dsw.specs.get(k).cloned())
        .collect();
    let to_unmount: Vec<VolumeKey> = have.difference(&want).cloned().collect();
    ReconcilePlan {
        to_mount,
        to_unmount,
    }
}

/// Apply the plan: simulate mount/unmount, updating ASW.
pub fn apply_plan(plan: &ReconcilePlan, asw: &mut ActualStateOfWorld) -> usize {
    let mut applied = 0;
    for spec in &plan.to_mount {
        let device = format!("/dev/{}/{}", spec.plugin, spec.key.volume_name);
        let _ = asw.record_attach(&device);
        asw.record_mount(MountRecord {
            key: spec.key.clone(),
            device_path: device.clone(),
            mount_path: format!("/var/lib/kubelet/pods/{}/{}", spec.key.pod_uid, spec.key.volume_name),
            mounted_at: Utc::now(),
        });
        applied += 1;
    }
    for key in &plan.to_unmount {
        if let Ok(rec) = asw.record_unmount(key) {
            asw.record_detach(&rec.device_path);
            applied += 1;
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pod: Uuid, name: &str, tenant: &str) -> VolumeKey {
        VolumeKey {
            pod_uid: pod,
            volume_name: name.into(),
            tenant_id: tenant.into(),
        }
    }

    fn spec(k: VolumeKey, plugin: &str) -> VolumeSpec {
        VolumeSpec {
            key: k,
            plugin: plugin.into(),
            fs_type: "ext4".into(),
            read_only: false,
        }
    }

    #[test]
    fn empty_dsw_empty_asw_no_actions() {
        let dsw = DesiredStateOfWorld::default();
        let asw = ActualStateOfWorld::default();
        let plan = reconcile(&dsw, &asw);
        assert!(plan.to_mount.is_empty());
        assert!(plan.to_unmount.is_empty());
    }

    #[test]
    fn dsw_only_yields_mount() {
        let pod = Uuid::new_v4();
        let mut dsw = DesiredStateOfWorld::default();
        dsw.add_pod_volume(spec(key(pod, "data", "acme"), "csi-pd"));
        let asw = ActualStateOfWorld::default();
        let plan = reconcile(&dsw, &asw);
        assert_eq!(plan.to_mount.len(), 1);
        assert!(plan.to_unmount.is_empty());
    }

    #[test]
    fn asw_only_yields_unmount() {
        let pod = Uuid::new_v4();
        let dsw = DesiredStateOfWorld::default();
        let mut asw = ActualStateOfWorld::default();
        asw.record_mount(MountRecord {
            key: key(pod, "data", "acme"),
            device_path: "/dev/csi-pd/data".into(),
            mount_path: "/x".into(),
            mounted_at: Utc::now(),
        });
        let plan = reconcile(&dsw, &asw);
        assert_eq!(plan.to_unmount.len(), 1);
    }

    #[test]
    fn matched_state_is_no_op() {
        let pod = Uuid::new_v4();
        let k = key(pod, "data", "acme");
        let mut dsw = DesiredStateOfWorld::default();
        dsw.add_pod_volume(spec(k.clone(), "csi-pd"));
        let mut asw = ActualStateOfWorld::default();
        asw.record_mount(MountRecord {
            key: k.clone(),
            device_path: "/dev/x".into(),
            mount_path: "/y".into(),
            mounted_at: Utc::now(),
        });
        let plan = reconcile(&dsw, &asw);
        assert!(plan.to_mount.is_empty());
        assert!(plan.to_unmount.is_empty());
    }

    #[test]
    fn apply_plan_mounts_and_unmounts() {
        let pod = Uuid::new_v4();
        let mut dsw = DesiredStateOfWorld::default();
        dsw.add_pod_volume(spec(key(pod, "v1", "t"), "csi-pd"));
        let mut asw = ActualStateOfWorld::default();
        let plan = reconcile(&dsw, &asw);
        let n = apply_plan(&plan, &mut asw);
        assert_eq!(n, 1);
        assert_eq!(asw.mounted.len(), 1);
    }

    #[test]
    fn cross_tenant_keys_do_not_collide() {
        let pod = Uuid::new_v4();
        let mut dsw = DesiredStateOfWorld::default();
        dsw.add_pod_volume(spec(key(pod, "v", "acme"), "csi-pd"));
        dsw.add_pod_volume(spec(key(pod, "v", "rival"), "csi-pd"));
        assert_eq!(dsw.specs.len(), 2);
        let asw = ActualStateOfWorld::default();
        let plan = reconcile(&dsw, &asw);
        assert_eq!(plan.to_mount.len(), 2);
    }

    #[test]
    fn double_attach_is_rejected() {
        let mut asw = ActualStateOfWorld::default();
        asw.record_attach("/dev/sda").unwrap();
        assert!(matches!(
            asw.record_attach("/dev/sda"),
            Err(VolumeError::AlreadyAttached(_))
        ));
    }

    #[test]
    fn unmount_unknown_errors() {
        let mut asw = ActualStateOfWorld::default();
        let pod = Uuid::new_v4();
        assert!(matches!(
            asw.record_unmount(&key(pod, "ghost", "t")),
            Err(VolumeError::NotMounted(_))
        ));
    }

    #[test]
    fn full_round_trip_drains_asw() {
        let pod = Uuid::new_v4();
        let mut dsw = DesiredStateOfWorld::default();
        let k = key(pod, "v", "acme");
        dsw.add_pod_volume(spec(k.clone(), "csi-pd"));
        let mut asw = ActualStateOfWorld::default();
        let plan = reconcile(&dsw, &asw);
        apply_plan(&plan, &mut asw);
        // Pod deleted → DSW removes the entry → reconcile produces unmount
        dsw.remove_pod_volume(&k);
        let plan2 = reconcile(&dsw, &asw);
        assert_eq!(plan2.to_unmount.len(), 1);
        apply_plan(&plan2, &mut asw);
        assert!(asw.mounted.is_empty());
        assert!(asw.attached_devices.is_empty());
    }
}
