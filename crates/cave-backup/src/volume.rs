// SPDX-License-Identifier: AGPL-3.0-or-later
//! Volume snapshot helpers and CSI integration types.

use crate::types::{FileBackupTool, VolumeSnapshot, VolumeSnapshotConfig};
use chrono::Utc;

/// Returns true if all snapshots in the list are ready.
pub fn all_snapshots_ready(snapshots: &[VolumeSnapshot]) -> bool {
    snapshots.iter().all(|s| s.ready)
}

/// Returns snapshots that are not yet ready.
pub fn pending_snapshots(snapshots: &[VolumeSnapshot]) -> Vec<&VolumeSnapshot> {
    snapshots.iter().filter(|s| !s.ready).collect()
}

/// Build a snapshot name following Velero convention: `velero-<pvc>-<timestamp>`.
pub fn snapshot_name(pvc_name: &str) -> String {
    let ts = Utc::now().format("%Y%m%d%H%M%S");
    format!("cave-{pvc_name}-{ts}")
}

/// Choose the effective backup tool for volumes.
pub fn effective_backup_tool(config: &VolumeSnapshotConfig) -> Option<&FileBackupTool> {
    if config.use_file_backup {
        Some(&config.file_backup_tool)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_snapshot(ready: bool) -> VolumeSnapshot {
        VolumeSnapshot {
            name: "snap-1".into(),
            namespace: "default".into(),
            pvc_name: "data-pvc".into(),
            snapshot_class: "csi-snapclass".into(),
            creation_time: Utc::now(),
            restore_size_bytes: 1024 * 1024 * 1024,
            ready,
        }
    }

    #[test]
    fn test_all_snapshots_ready_true() {
        let snaps = vec![make_snapshot(true), make_snapshot(true)];
        assert!(all_snapshots_ready(&snaps));
    }

    #[test]
    fn test_all_snapshots_ready_false() {
        let snaps = vec![make_snapshot(true), make_snapshot(false)];
        assert!(!all_snapshots_ready(&snaps));
    }

    #[test]
    fn test_pending_snapshots() {
        let snaps = vec![make_snapshot(true), make_snapshot(false)];
        assert_eq!(pending_snapshots(&snaps).len(), 1);
    }

    #[test]
    fn test_snapshot_name_format() {
        let name = snapshot_name("my-pvc");
        assert!(name.starts_with("cave-my-pvc-"));
    }

    #[test]
    fn test_effective_backup_tool_csi() {
        let config = VolumeSnapshotConfig {
            enabled: true,
            snapshot_class: Some("csi-class".into()),
            use_file_backup: false,
            file_backup_tool: FileBackupTool::Restic,
        };
        assert!(effective_backup_tool(&config).is_none());
    }

    #[test]
    fn test_effective_backup_tool_restic() {
        let config = VolumeSnapshotConfig {
            enabled: true,
            snapshot_class: None,
            use_file_backup: true,
            file_backup_tool: FileBackupTool::Restic,
        };
        assert!(matches!(
            effective_backup_tool(&config),
            Some(FileBackupTool::Restic)
        ));
    }
}
