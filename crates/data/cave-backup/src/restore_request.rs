// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Restore-request validation engine.
//!
//! Pure port of the deterministic validation in Velero's
//! `pkg/controller/restore_controller.go` `validateAndComplete`,
//! `backupXorScheduleProvided`, and `mostRecentCompletedBackup` (Apache-2.0).
//! The controller-runtime reconcile loop and the Kubernetes client lookups
//! (backup/BSL fetch) stay owned by cave-controller-manager; only the
//! runtime-free business logic is line-ported here.

use crate::includes_excludes::validate_includes_excludes;

/// Resources that can never be restored. Mirrors Velero's
/// `nonRestorableResources()`.
pub const NON_RESTORABLE_RESOURCES: &[&str] = &[
    "nodes",
    "events",
    "events.events.k8s.io",
    "backups.velero.io",
    "restores.velero.io",
    "resticrepositories.velero.io",
    "csinodes.storage.k8s.io",
    "volumeattachments.storage.k8s.io",
    "backuprepositories.velero.io",
];

/// The subset of a Velero `RestoreSpec` that participates in validation.
#[derive(Debug, Clone, Default)]
pub struct RestoreRequestSpec {
    pub backup_name: String,
    pub schedule_name: String,
    pub included_namespaces: Vec<String>,
    pub excluded_namespaces: Vec<String>,
    pub included_resources: Vec<String>,
    pub excluded_resources: Vec<String>,
}

/// A backup candidate used when resolving a schedule to its most recent
/// completed backup.
#[derive(Debug, Clone)]
pub struct BackupRef {
    pub name: String,
    pub start_timestamp: i64,
    pub completed: bool,
}

/// True iff exactly one of `backup_name` / `schedule_name` is non-empty.
/// Mirrors Velero `backupXorScheduleProvided`.
pub fn backup_xor_schedule_provided(backup_name: &str, schedule_name: &str) -> bool {
    !backup_name.is_empty() && schedule_name.is_empty()
        || backup_name.is_empty() && !schedule_name.is_empty()
}

/// Return the name of the most recent completed backup (highest
/// `start_timestamp` among `completed == true`), or `None`. Mirrors Velero
/// `mostRecentCompletedBackup` (descending sort on StartTimestamp, first
/// completed).
pub fn most_recent_completed_backup(backups: &[BackupRef]) -> Option<String> {
    backups
        .iter()
        .filter(|b| b.completed)
        .max_by_key(|b| b.start_timestamp)
        .map(|b| b.name.clone())
}

/// Validate a restore request, returning the accumulated validation errors
/// (mirrors appending to `restore.Status.ValidationErrors`). Empty == valid.
pub fn validate_restore_request(spec: &RestoreRequestSpec) -> Vec<String> {
    let mut errs = Vec::new();

    // Included resources must not name a non-restorable resource.
    for res in &spec.included_resources {
        if NON_RESTORABLE_RESOURCES.contains(&res.as_str()) {
            errs.push(format!("{res} are non-restorable resources"));
        }
    }

    // Resource include/exclude overlap.
    let res = validate_includes_excludes(&spec.included_resources, &spec.excluded_resources);
    if !res.is_empty() {
        errs.push(format!("Invalid included/excluded resource lists: {}", res.join("; ")));
    }

    // Namespace include/exclude overlap.
    let ns = validate_includes_excludes(&spec.included_namespaces, &spec.excluded_namespaces);
    if !ns.is_empty() {
        errs.push(format!("Invalid included/excluded namespace lists: {}", ns.join("; ")));
    }

    // Exactly one source must be specified.
    if !backup_xor_schedule_provided(&spec.backup_name, &spec.schedule_name) {
        errs.push(
            "Either a backup or schedule must be specified as a source for the restore, but not both"
                .to_string(),
        );
    }

    errs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn most_recent_breaks_ties_deterministically() {
        // Equal completed timestamps — max_by_key returns the last max element.
        let backups = vec![
            BackupRef { name: "a".into(), start_timestamp: 50, completed: true },
            BackupRef { name: "b".into(), start_timestamp: 50, completed: true },
        ];
        assert!(most_recent_completed_backup(&backups).is_some());
    }

    #[test]
    fn non_restorable_constant_includes_nodes() {
        assert!(NON_RESTORABLE_RESOURCES.contains(&"nodes"));
        assert!(NON_RESTORABLE_RESOURCES.contains(&"backups.velero.io"));
    }
}
