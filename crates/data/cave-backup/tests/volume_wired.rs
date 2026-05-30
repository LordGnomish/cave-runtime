// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Guards that the `volume` module (internal/volume.go port) is actually
//! wired into the crate's public surface — it was previously present on disk
//! but undeclared in lib.rs, so its 6 mapped tests never compiled.

use cave_backup::types::{FileBackupTool, VolumeSnapshot, VolumeSnapshotConfig};
use cave_backup::volume;
use chrono::Utc;

#[test]
fn volume_module_is_publicly_reachable() {
    // snapshot_name follows the cave-<pvc>-<ts> convention.
    let name = volume::snapshot_name("data-pvc");
    assert!(name.starts_with("cave-data-pvc-"));

    // all_snapshots_ready / pending_snapshots operate over the public type.
    let snaps = vec![
        VolumeSnapshot {
            name: "snap-1".into(),
            namespace: "default".into(),
            pvc_name: "data-pvc".into(),
            snapshot_class: "csi-snapclass".into(),
            creation_time: Utc::now(),
            restore_size_bytes: 1024,
            ready: true,
        },
        VolumeSnapshot {
            name: "snap-2".into(),
            namespace: "default".into(),
            pvc_name: "log-pvc".into(),
            snapshot_class: "csi-snapclass".into(),
            creation_time: Utc::now(),
            restore_size_bytes: 1024,
            ready: false,
        },
    ];
    assert!(!volume::all_snapshots_ready(&snaps));
    assert_eq!(volume::pending_snapshots(&snaps).len(), 1);

    // effective_backup_tool returns None for CSI-only configs.
    let csi = VolumeSnapshotConfig {
        enabled: true,
        snapshot_class: Some("csi-class".into()),
        use_file_backup: false,
        file_backup_tool: FileBackupTool::Restic,
    };
    assert!(volume::effective_backup_tool(&csi).is_none());
}
