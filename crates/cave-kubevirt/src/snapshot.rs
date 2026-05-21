// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! VirtualMachineSnapshot + VirtualMachineRestore CRDs.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   staging/src/kubevirt.io/api/snapshot/v1beta1/types.go
//!   pkg/storage/snapshot/snapshot.go
//!
//! Snapshots freeze the disk state of a VM at a point in time, leveraging
//! the underlying VolumeSnapshot CSI primitive. Restores create a new VM
//! by replaying a snapshot's PVCs. This module captures the CRDs, the
//! phase enums, and the controller reconcile predicates.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotPhase {
    Pending,
    InProgress,
    Succeeded,
    Failed,
    Deleting,
}

impl SnapshotPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, SnapshotPhase::Succeeded | SnapshotPhase::Failed)
    }

    pub fn allows_restore(&self) -> bool {
        matches!(self, SnapshotPhase::Succeeded)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestorePhase {
    Pending,
    InProgress,
    Succeeded,
    Failed,
}

impl RestorePhase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, RestorePhase::Succeeded | RestorePhase::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualMachineSnapshot {
    pub name: String,
    pub namespace: Option<String>,
    pub spec: SnapshotSpec,
    pub status: Option<SnapshotStatus>,
}

impl Default for VirtualMachineSnapshot {
    fn default() -> Self {
        Self {
            name: String::new(),
            namespace: None,
            spec: SnapshotSpec::default(),
            status: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSpec {
    /// Name of the source VM in the same namespace.
    pub source_vm: String,
    /// Optional deadline for the snapshot to be marked failed if not
    /// completed.
    pub deadline_seconds: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotStatus {
    pub phase: Option<SnapshotPhase>,
    pub creation_timestamp_unix: Option<i64>,
    pub completion_timestamp_unix: Option<i64>,
    /// CSI VolumeSnapshot names this Kubevirt snapshot wraps.
    pub volume_snapshot_names: Vec<String>,
    pub ready_to_use: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualMachineRestore {
    pub name: String,
    pub namespace: Option<String>,
    pub spec: RestoreSpec,
    pub status: Option<RestoreStatus>,
}

impl Default for VirtualMachineRestore {
    fn default() -> Self {
        Self {
            name: String::new(),
            namespace: None,
            spec: RestoreSpec::default(),
            status: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RestoreSpec {
    pub snapshot_name: String,
    pub target_vm: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RestoreStatus {
    pub phase: Option<RestorePhase>,
    pub restored_pvc_names: Vec<String>,
    pub completion_timestamp_unix: Option<i64>,
}

/// Predicate: is this snapshot deadline-expired given a "now"?
pub fn deadline_expired(snap: &VirtualMachineSnapshot, now_unix: i64) -> bool {
    let Some(deadline) = snap.spec.deadline_seconds else {
        return false;
    };
    let Some(status) = snap.status.as_ref() else {
        return false;
    };
    let Some(created) = status.creation_timestamp_unix else {
        return false;
    };
    if let Some(phase) = status.phase {
        if phase.is_terminal() {
            return false;
        }
    }
    now_unix.saturating_sub(created) > deadline as i64
}

/// Predicate: can this restore proceed given the referenced snapshot?
pub fn restore_can_proceed(
    restore: &VirtualMachineRestore,
    snapshot: Option<&VirtualMachineSnapshot>,
) -> bool {
    let Some(snap) = snapshot else {
        return false;
    };
    if snap.name != restore.spec.snapshot_name {
        return false;
    }
    snap.status
        .as_ref()
        .and_then(|s| s.phase)
        .map(|p| p.allows_restore())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(phase: Option<SnapshotPhase>, created: Option<i64>) -> VirtualMachineSnapshot {
        VirtualMachineSnapshot {
            name: "snap-1".into(),
            namespace: Some("default".into()),
            spec: SnapshotSpec {
                source_vm: "vm-1".into(),
                deadline_seconds: Some(60),
            },
            status: Some(SnapshotStatus {
                phase,
                creation_timestamp_unix: created,
                ready_to_use: false,
                ..Default::default()
            }),
        }
    }

    #[test]
    fn snapshot_terminal_phases() {
        assert!(SnapshotPhase::Succeeded.is_terminal());
        assert!(SnapshotPhase::Failed.is_terminal());
        assert!(!SnapshotPhase::Pending.is_terminal());
        assert!(!SnapshotPhase::InProgress.is_terminal());
    }

    #[test]
    fn only_succeeded_allows_restore() {
        assert!(SnapshotPhase::Succeeded.allows_restore());
        assert!(!SnapshotPhase::Failed.allows_restore());
        assert!(!SnapshotPhase::Pending.allows_restore());
    }

    #[test]
    fn restore_terminal_phases() {
        assert!(RestorePhase::Succeeded.is_terminal());
        assert!(RestorePhase::Failed.is_terminal());
        assert!(!RestorePhase::Pending.is_terminal());
    }

    #[test]
    fn deadline_expired_when_too_old() {
        let s = snap(Some(SnapshotPhase::InProgress), Some(1000));
        // deadline=60, now=1100, 100 > 60 → expired.
        assert!(deadline_expired(&s, 1100));
    }

    #[test]
    fn deadline_not_expired_within_window() {
        let s = snap(Some(SnapshotPhase::InProgress), Some(1000));
        assert!(!deadline_expired(&s, 1010));
    }

    #[test]
    fn deadline_does_not_apply_to_terminal_snapshots() {
        let s = snap(Some(SnapshotPhase::Succeeded), Some(1000));
        assert!(!deadline_expired(&s, 2000));
    }

    #[test]
    fn deadline_missing_returns_false() {
        let mut s = snap(Some(SnapshotPhase::InProgress), Some(1000));
        s.spec.deadline_seconds = None;
        assert!(!deadline_expired(&s, 99999));
    }

    #[test]
    fn restore_blocks_without_snapshot() {
        let r = VirtualMachineRestore {
            name: "r1".into(),
            namespace: Some("default".into()),
            spec: RestoreSpec {
                snapshot_name: "snap-1".into(),
                target_vm: "vm-2".into(),
            },
            status: None,
        };
        assert!(!restore_can_proceed(&r, None));
    }

    #[test]
    fn restore_blocks_on_wrong_snapshot() {
        let r = VirtualMachineRestore {
            name: "r1".into(),
            namespace: Some("default".into()),
            spec: RestoreSpec {
                snapshot_name: "snap-other".into(),
                target_vm: "vm-2".into(),
            },
            status: None,
        };
        let s = snap(Some(SnapshotPhase::Succeeded), Some(1000));
        assert!(!restore_can_proceed(&r, Some(&s)));
    }

    #[test]
    fn restore_blocks_on_unfinished_snapshot() {
        let r = VirtualMachineRestore {
            name: "r1".into(),
            namespace: Some("default".into()),
            spec: RestoreSpec {
                snapshot_name: "snap-1".into(),
                target_vm: "vm-2".into(),
            },
            status: None,
        };
        let s = snap(Some(SnapshotPhase::InProgress), Some(1000));
        assert!(!restore_can_proceed(&r, Some(&s)));
    }

    #[test]
    fn restore_proceeds_on_succeeded_snapshot() {
        let r = VirtualMachineRestore {
            name: "r1".into(),
            namespace: Some("default".into()),
            spec: RestoreSpec {
                snapshot_name: "snap-1".into(),
                target_vm: "vm-2".into(),
            },
            status: None,
        };
        let s = snap(Some(SnapshotPhase::Succeeded), Some(1000));
        assert!(restore_can_proceed(&r, Some(&s)));
    }

    #[test]
    fn serde_round_trip_snapshot() {
        let s = snap(Some(SnapshotPhase::Succeeded), Some(1000));
        let json = serde_json::to_string(&s).unwrap();
        let back: VirtualMachineSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn serde_round_trip_restore() {
        let r = VirtualMachineRestore {
            name: "r1".into(),
            namespace: Some("default".into()),
            spec: RestoreSpec {
                snapshot_name: "snap-1".into(),
                target_vm: "vm-2".into(),
            },
            status: Some(RestoreStatus {
                phase: Some(RestorePhase::Succeeded),
                restored_pvc_names: vec!["pvc-a".into(), "pvc-b".into()],
                completion_timestamp_unix: Some(2000),
            }),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: VirtualMachineRestore = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}
