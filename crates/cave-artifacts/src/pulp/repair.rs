// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: pulp/pulpcore@0f991c2fa2bf6c8635e8a2de064ef04dacbbcf4f pulpcore/app/tasks/repair.py
//! Repair — reclaim space, verify checksums, re-download missing content.

use crate::pulp::models::Artifact;
use crate::pulp::tasks::{Task, TaskQueue};

#[derive(Debug, Clone)]
pub struct RepairOptions {
    pub verify_checksums: bool,
    pub redownload_missing: bool,
    pub dry_run: bool,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            verify_checksums: true,
            redownload_missing: true,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepairReport {
    pub total_checked: u64,
    pub missing: u64,
    pub corrupted: u64,
    pub repaired: u64,
    pub unrepairable: u64,
    pub space_reclaimed_bytes: u64,
}

impl RepairReport {
    pub fn is_clean(&self) -> bool {
        self.missing == 0 && self.corrupted == 0
    }
}

/// Enqueue a repair task.
pub fn enqueue_repair(
    repo_version_href: &str,
    options: &RepairOptions,
    queue: &TaskQueue,
) -> Task {
    let task = queue.enqueue("pulp.tasks.repair");
    tracing::info!(
        repo_version = %repo_version_href,
        verify_checksums = options.verify_checksums,
        dry_run = options.dry_run,
        task_id = %task.pulp_id,
        "Repair task enqueued"
    );
    task
}

/// Check an artifact's local existence and checksum.
pub fn check_artifact(artifact: &Artifact, data: Option<&[u8]>) -> ArtifactCheck {
    match data {
        None => ArtifactCheck::Missing,
        Some(d) => {
            if let Some(ref expected_sha256) = artifact.sha256 {
                // Simplified: verify length plausibility (real = sha2 crate)
                if expected_sha256.len() == 64 && d.len() as u64 == artifact.size {
                    ArtifactCheck::Ok
                } else {
                    ArtifactCheck::Corrupted {
                        expected_sha256: expected_sha256.clone(),
                        actual_size: d.len() as u64,
                    }
                }
            } else {
                ArtifactCheck::Ok
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArtifactCheck {
    Ok,
    Missing,
    Corrupted { expected_sha256: String, actual_size: u64 },
}

impl ArtifactCheck {
    pub fn needs_repair(&self) -> bool {
        !matches!(self, Self::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pulp::models::Artifact;
    use uuid::Uuid;
    use chrono::Utc;

    fn make_artifact(size: u64, sha256: &str) -> Artifact {
        let id = Uuid::new_v4();
        Artifact {
            pulp_href: format!("/pulp/api/v3/artifacts/{}/", id),
            pulp_id: id,
            pulp_created: Utc::now(),
            file: format!("/var/lib/pulp/artifacts/{}", id),
            size,
            md5: None,
            sha1: None,
            sha224: None,
            sha256: Some(sha256.to_string()),
            sha384: None,
            sha512: None,
            timestamp_of_interest: None,
        }
    }

    #[test]
    fn artifact_check_missing() {
        let artifact = make_artifact(1024, &"a".repeat(64));
        let check = check_artifact(&artifact, None);
        assert_eq!(check, ArtifactCheck::Missing);
        assert!(check.needs_repair());
    }

    #[test]
    fn artifact_check_ok() {
        let data = vec![0u8; 1024];
        let artifact = make_artifact(1024, &"a".repeat(64));
        let check = check_artifact(&artifact, Some(&data));
        assert_eq!(check, ArtifactCheck::Ok);
        assert!(!check.needs_repair());
    }

    #[test]
    fn artifact_check_corrupted_size_mismatch() {
        let data = vec![0u8; 512]; // wrong size
        let artifact = make_artifact(1024, &"a".repeat(64));
        let check = check_artifact(&artifact, Some(&data));
        assert!(matches!(check, ArtifactCheck::Corrupted { .. }));
        assert!(check.needs_repair());
    }

    #[test]
    fn repair_report_clean() {
        let report = RepairReport {
            total_checked: 100,
            missing: 0,
            corrupted: 0,
            repaired: 0,
            unrepairable: 0,
            space_reclaimed_bytes: 0,
        };
        assert!(report.is_clean());
    }

    #[test]
    fn repair_report_not_clean() {
        let report = RepairReport {
            total_checked: 100,
            missing: 3,
            corrupted: 1,
            repaired: 2,
            unrepairable: 2,
            space_reclaimed_bytes: 0,
        };
        assert!(!report.is_clean());
    }

    #[test]
    fn enqueue_repair_task() {
        let queue = TaskQueue::new();
        let opts = RepairOptions::default();
        let task = enqueue_repair("/pulp/api/v3/repositories/abc/versions/5/", &opts, &queue);
        assert_eq!(task.name, "pulp.tasks.repair");
    }
}
