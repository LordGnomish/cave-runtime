// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Mirror tests for the backup-request preparation/validation engine ported
//! from Velero's pkg/controller/backup_controller.go `prepareBackupRequest`.
//! Error strings are reproduced verbatim from upstream (Apache-2.0).

use cave_backup::backup_request::{prepare_backup_request, BackupDefaults, BackupRequestSpec};

fn defaults() -> BackupDefaults {
    BackupDefaults {
        ttl_hours: 720,                  // Velero defaultBackupTTL = 30 * 24h
        csi_snapshot_timeout_minutes: 10, // Velero defaultCSISnapshotTimeout
        item_operation_timeout_minutes: 240,
    }
}

// ── Defaulting ──────────────────────────────────────────────────────────────

#[test]
fn applies_defaults_when_zero() {
    let mut spec = BackupRequestSpec::default();
    spec.ttl_hours = 0;
    spec.csi_snapshot_timeout_minutes = 0;
    spec.item_operation_timeout_minutes = 0;
    spec.included_namespaces = vec![];

    let errs = prepare_backup_request(&mut spec, &defaults());

    assert!(errs.is_empty(), "expected no validation errors, got {errs:?}");
    assert_eq!(spec.ttl_hours, 720);
    assert_eq!(spec.csi_snapshot_timeout_minutes, 10);
    assert_eq!(spec.item_operation_timeout_minutes, 240);
    // IncludedNamespaces defaults to ["*"]
    assert_eq!(spec.included_namespaces, vec!["*".to_string()]);
}

#[test]
fn preserves_nonzero_values() {
    let mut spec = BackupRequestSpec::default();
    spec.ttl_hours = 48;
    spec.csi_snapshot_timeout_minutes = 5;
    spec.item_operation_timeout_minutes = 60;
    spec.included_namespaces = vec!["prod".into()];

    let errs = prepare_backup_request(&mut spec, &defaults());
    assert!(errs.is_empty());
    assert_eq!(spec.ttl_hours, 48);
    assert_eq!(spec.csi_snapshot_timeout_minutes, 5);
    assert_eq!(spec.item_operation_timeout_minutes, 60);
    assert_eq!(spec.included_namespaces, vec!["prod".to_string()]);
}

// ── Validation ──────────────────────────────────────────────────────────────

#[test]
fn rejects_namespace_overlap() {
    let mut spec = BackupRequestSpec::default();
    spec.included_namespaces = vec!["a".into(), "b".into()];
    spec.excluded_namespaces = vec!["b".into()];

    let errs = prepare_backup_request(&mut spec, &defaults());
    assert!(
        errs.iter().any(|e| e.starts_with("Invalid included/excluded namespace lists: ")),
        "got {errs:?}"
    );
}

#[test]
fn rejects_resource_overlap_old_filters() {
    let mut spec = BackupRequestSpec::default();
    spec.included_resources = vec!["pods".into(), "secrets".into()];
    spec.excluded_resources = vec!["secrets".into()];

    let errs = prepare_backup_request(&mut spec, &defaults());
    assert!(
        errs.iter().any(|e| e.starts_with("Invalid included/excluded resource lists: ")),
        "got {errs:?}"
    );
}

#[test]
fn rejects_old_and_new_filters_together() {
    let mut spec = BackupRequestSpec::default();
    spec.included_resources = vec!["pods".into()]; // old filter
    spec.included_cluster_scoped_resources = vec!["persistentvolumes".into()]; // new filter

    let errs = prepare_backup_request(&mut spec, &defaults());
    assert!(
        errs.iter().any(|e| e == "include-resources, exclude-resources and include-cluster-resources are old filter parameters.\ninclude-cluster-scoped-resources, exclude-cluster-scoped-resources, include-namespace-scoped-resources and exclude-namespace-scoped-resources are new filter parameters.\nThey cannot be used together"),
        "got {errs:?}"
    );
}

#[test]
fn rejects_label_and_or_label_selectors_together() {
    let mut spec = BackupRequestSpec::default();
    spec.label_selector = Some("app=web".into());
    spec.or_label_selectors = vec!["tier=db".into()];

    let errs = prepare_backup_request(&mut spec, &defaults());
    assert!(
        errs.iter().any(|e| e
            == "encountered labelSelector as well as orLabelSelectors in backup spec, only one can be specified"),
        "got {errs:?}"
    );
}

#[test]
fn rejects_old_filters_with_resource_policies() {
    let mut spec = BackupRequestSpec::default();
    spec.included_resources = vec!["pods".into()];
    spec.has_resource_policies = true;

    let errs = prepare_backup_request(&mut spec, &defaults());
    assert!(
        errs.iter().any(|e| e == "include-resources, exclude-resources and include-cluster-resources are old filter parameters.\nThey cannot be used with include-exclude policies."),
        "got {errs:?}"
    );
}

#[test]
fn rejects_new_cluster_scoped_overlap() {
    let mut spec = BackupRequestSpec::default();
    spec.included_cluster_scoped_resources = vec!["a".into(), "b".into()];
    spec.excluded_cluster_scoped_resources = vec!["b".into()];

    let errs = prepare_backup_request(&mut spec, &defaults());
    assert!(
        errs.iter().any(|e| e.starts_with("Invalid cluster-scoped included/excluded resource lists: ")),
        "got {errs:?}"
    );
}

#[test]
fn clean_spec_has_no_errors() {
    let mut spec = BackupRequestSpec::default();
    spec.included_namespaces = vec!["prod".into()];
    spec.included_resources = vec!["pods".into(), "deployments".into()];
    let errs = prepare_backup_request(&mut spec, &defaults());
    assert!(errs.is_empty(), "got {errs:?}");
}
