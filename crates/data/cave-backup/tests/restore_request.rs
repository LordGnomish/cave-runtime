// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Mirror tests for the restore-request validation logic ported from Velero
//! pkg/controller/restore_controller.go `validateAndComplete` /
//! `backupXorScheduleProvided` / `mostRecentCompletedBackup`. Error strings
//! verbatim from upstream (Apache-2.0).

use cave_backup::restore_request::{
    backup_xor_schedule_provided, most_recent_completed_backup, validate_restore_request,
    BackupRef, RestoreRequestSpec,
};

// ── BackupName XOR ScheduleName ──────────────────────────────────────────────

#[test]
fn xor_true_for_backup_only() {
    assert!(backup_xor_schedule_provided("daily-1", ""));
}

#[test]
fn xor_true_for_schedule_only() {
    assert!(backup_xor_schedule_provided("", "daily"));
}

#[test]
fn xor_false_for_both() {
    assert!(!backup_xor_schedule_provided("daily-1", "daily"));
}

#[test]
fn xor_false_for_neither() {
    assert!(!backup_xor_schedule_provided("", ""));
}

#[test]
fn validate_rejects_both_source_specified() {
    let mut spec = RestoreRequestSpec::default();
    spec.backup_name = "daily-1".into();
    spec.schedule_name = "daily".into();
    let errs = validate_restore_request(&spec);
    assert!(
        errs.iter().any(|e| e
            == "Either a backup or schedule must be specified as a source for the restore, but not both"),
        "got {errs:?}"
    );
}

#[test]
fn validate_accepts_backup_only() {
    let mut spec = RestoreRequestSpec::default();
    spec.backup_name = "daily-1".into();
    let errs = validate_restore_request(&spec);
    assert!(errs.is_empty(), "got {errs:?}");
}

// ── Non-restorable resources ─────────────────────────────────────────────────

#[test]
fn validate_rejects_non_restorable_resource() {
    let mut spec = RestoreRequestSpec::default();
    spec.backup_name = "daily-1".into();
    spec.included_resources = vec!["nodes".into()];
    let errs = validate_restore_request(&spec);
    assert!(
        errs.iter().any(|e| e == "nodes are non-restorable resources"),
        "got {errs:?}"
    );
}

// ── Overlap validation ───────────────────────────────────────────────────────

#[test]
fn validate_rejects_resource_overlap() {
    let mut spec = RestoreRequestSpec::default();
    spec.backup_name = "daily-1".into();
    spec.included_resources = vec!["pods".into(), "secrets".into()];
    spec.excluded_resources = vec!["secrets".into()];
    let errs = validate_restore_request(&spec);
    assert!(
        errs.iter().any(|e| e.starts_with("Invalid included/excluded resource lists: ")),
        "got {errs:?}"
    );
}

#[test]
fn validate_rejects_namespace_overlap() {
    let mut spec = RestoreRequestSpec::default();
    spec.backup_name = "daily-1".into();
    spec.included_namespaces = vec!["a".into(), "b".into()];
    spec.excluded_namespaces = vec!["b".into()];
    let errs = validate_restore_request(&spec);
    assert!(
        errs.iter().any(|e| e.starts_with("Invalid included/excluded namespace lists: ")),
        "got {errs:?}"
    );
}

// ── mostRecentCompletedBackup ────────────────────────────────────────────────

#[test]
fn most_recent_completed_picks_latest_completed() {
    let backups = vec![
        BackupRef { name: "b-old".into(), start_timestamp: 100, completed: true },
        BackupRef { name: "b-new-failed".into(), start_timestamp: 300, completed: false },
        BackupRef { name: "b-mid".into(), start_timestamp: 200, completed: true },
    ];
    assert_eq!(most_recent_completed_backup(&backups), Some("b-mid".to_string()));
}

#[test]
fn most_recent_completed_none_when_no_completed() {
    let backups = vec![
        BackupRef { name: "b1".into(), start_timestamp: 100, completed: false },
        BackupRef { name: "b2".into(), start_timestamp: 200, completed: false },
    ];
    assert_eq!(most_recent_completed_backup(&backups), None);
}

#[test]
fn most_recent_completed_empty() {
    assert_eq!(most_recent_completed_backup(&[]), None);
}
