// SPDX-License-Identifier: AGPL-3.0-or-later
//! Filesystem backup (restic/kopia) job management.

use crate::models::{FsBackupJob, FsBackupMethod};
use uuid::Uuid;

/// Create a new filesystem backup job in the InProgress phase.
pub fn create_fs_backup_job(
    backup_id: Uuid,
    method: FsBackupMethod,
    namespace: &str,
    pod: &str,
    volume: &str,
) -> FsBackupJob {
    use crate::models::BackupPhase;
    FsBackupJob {
        id: Uuid::new_v4(),
        backup_id,
        method,
        namespace: namespace.to_string(),
        pod: pod.to_string(),
        volume: volume.to_string(),
        phase: BackupPhase::InProgress,
        snapshot_id: None,
        size_bytes: 0,
        started_at: Some(chrono::Utc::now()),
        completed_at: None,
    }
}

/// Complete a filesystem backup job with its snapshot ID and size.
pub fn complete_fs_backup(job: &mut FsBackupJob, snapshot_id: String, size_bytes: u64) {
    use crate::models::BackupPhase;
    job.phase = BackupPhase::Completed;
    job.snapshot_id = Some(snapshot_id);
    job.size_bytes = size_bytes;
    job.completed_at = Some(chrono::Utc::now());
}

/// Return a human-readable description of the filesystem backup method.
pub fn method_description(method: &FsBackupMethod) -> &'static str {
    match method {
        FsBackupMethod::Restic => "restic: content-addressed deduplicating backup",
        FsBackupMethod::Kopia => "kopia: fast deduplicated backup with snapshot policies",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::BackupPhase;

    #[test]
    fn create_fs_backup_job_in_progress() {
        let backup_id = Uuid::new_v4();
        let job = create_fs_backup_job(
            backup_id,
            FsBackupMethod::Restic,
            "default",
            "my-pod",
            "data-vol",
        );
        assert_eq!(job.backup_id, backup_id);
        assert_eq!(job.phase, BackupPhase::InProgress);
        assert!(job.snapshot_id.is_none());
        assert_eq!(job.size_bytes, 0);
        assert!(job.started_at.is_some());
        assert!(job.completed_at.is_none());
        assert_eq!(job.namespace, "default");
        assert_eq!(job.pod, "my-pod");
        assert_eq!(job.volume, "data-vol");
    }

    #[test]
    fn complete_fs_backup_sets_completed() {
        let backup_id = Uuid::new_v4();
        let mut job =
            create_fs_backup_job(backup_id, FsBackupMethod::Kopia, "ns", "pod", "vol");
        complete_fs_backup(&mut job, "snap-abc123".into(), 2048);
        assert_eq!(job.phase, BackupPhase::Completed);
        assert_eq!(job.snapshot_id.as_deref(), Some("snap-abc123"));
        assert_eq!(job.size_bytes, 2048);
        assert!(job.completed_at.is_some());
    }

    #[test]
    fn method_description_restic() {
        let desc = method_description(&FsBackupMethod::Restic);
        assert!(desc.contains("restic"));
    }

    #[test]
    fn method_description_kopia() {
        let desc = method_description(&FsBackupMethod::Kopia);
        assert!(desc.contains("kopia"));
    }
}
