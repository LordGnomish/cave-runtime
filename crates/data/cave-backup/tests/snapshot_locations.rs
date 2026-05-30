// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Mirror tests for the BackupStorageLocation / VolumeSnapshotLocation
//! resolution logic ported from Velero pkg/controller/backup_controller.go
//! (validateAndGetSnapshotLocations + default BSL resolution). Error strings
//! verbatim from upstream (Apache-2.0).

use cave_backup::snapshot_locations::{
    resolve_default_backup_storage_location, validate_and_get_snapshot_locations,
    VolumeSnapshotLocationInfo as Vsl,
};
use std::collections::BTreeMap;

fn vsl(name: &str, provider: &str) -> Vsl {
    Vsl {
        name: name.into(),
        provider: provider.into(),
    }
}

// ── VolumeSnapshotLocation resolution ────────────────────────────────────────

#[test]
fn single_provider_location_auto_selected() {
    let available = vec![vsl("aws-default", "aws")];
    let resolved =
        validate_and_get_snapshot_locations(&[], &available, &BTreeMap::new()).unwrap();
    assert_eq!(resolved.get("aws"), Some(&"aws-default".to_string()));
}

#[test]
fn explicitly_named_location_resolved() {
    let available = vec![vsl("aws-1", "aws"), vsl("aws-2", "aws")];
    let resolved =
        validate_and_get_snapshot_locations(&["aws-2".into()], &available, &BTreeMap::new())
            .unwrap();
    assert_eq!(resolved.get("aws"), Some(&"aws-2".to_string()));
}

#[test]
fn duplicate_provider_in_spec_rejected() {
    let available = vec![vsl("aws-1", "aws"), vsl("aws-2", "aws")];
    let errs = validate_and_get_snapshot_locations(
        &["aws-1".into(), "aws-2".into()],
        &available,
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(
        errs.iter().any(|e| e
            == "more than one VolumeSnapshotLocation name specified for provider aws: aws-1; unexpected name was aws-2"),
        "got {errs:?}"
    );
}

#[test]
fn unknown_named_location_rejected() {
    let available = vec![vsl("aws-1", "aws")];
    let errs =
        validate_and_get_snapshot_locations(&["nosuch".into()], &available, &BTreeMap::new())
            .unwrap_err();
    assert!(
        errs.iter().any(|e| e
            == "a VolumeSnapshotLocation CRD for the location nosuch with the name specified in the backup spec needs to be created before this snapshot can be executed"),
        "got {errs:?}"
    );
}

#[test]
fn multiple_locations_no_default_rejected() {
    let available = vec![vsl("aws-1", "aws"), vsl("aws-2", "aws")];
    let errs =
        validate_and_get_snapshot_locations(&[], &available, &BTreeMap::new()).unwrap_err();
    assert!(
        errs.iter().any(|e| e
            == "provider aws has more than one possible volume snapshot location, and none were specified explicitly or as a default"),
        "got {errs:?}"
    );
}

#[test]
fn multiple_locations_default_used() {
    let available = vec![vsl("aws-1", "aws"), vsl("aws-2", "aws")];
    let mut defaults = BTreeMap::new();
    defaults.insert("aws".to_string(), "aws-2".to_string());
    let resolved =
        validate_and_get_snapshot_locations(&[], &available, &defaults).unwrap();
    assert_eq!(resolved.get("aws"), Some(&"aws-2".to_string()));
}

// ── default BackupStorageLocation resolution ─────────────────────────────────

#[test]
fn bsl_explicit_name_resolved() {
    let available = vec!["default".to_string(), "secondary".to_string()];
    let got = resolve_default_backup_storage_location(Some("secondary"), "default", &available)
        .unwrap();
    assert_eq!(got, "secondary");
}

#[test]
fn bsl_falls_back_to_server_default() {
    let available = vec!["default".to_string()];
    let got = resolve_default_backup_storage_location(None, "default", &available).unwrap();
    assert_eq!(got, "default");
}

#[test]
fn bsl_missing_server_default_rejected() {
    let available = vec!["other".to_string()];
    let err = resolve_default_backup_storage_location(None, "default", &available).unwrap_err();
    assert!(
        err.starts_with("an existing backup storage location was not specified at backup creation time and the server default default does not exist."),
        "got {err}"
    );
}
