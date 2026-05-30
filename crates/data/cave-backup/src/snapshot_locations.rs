// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! BackupStorageLocation / VolumeSnapshotLocation resolution.
//!
//! Pure port of Velero's `pkg/controller/backup_controller.go`
//! `validateAndGetSnapshotLocations` and the default-BackupStorageLocation
//! resolution path (Apache-2.0). Only the deterministic selection algebra is
//! ported — the controller-runtime client lookups stay owned by
//! cave-controller-manager; callers pass the already-listed locations in.

use std::collections::{BTreeMap, BTreeSet};

/// A VolumeSnapshotLocation, reduced to the fields selection depends on.
#[derive(Debug, Clone)]
pub struct VolumeSnapshotLocationInfo {
    pub name: String,
    pub provider: String,
}

/// Resolve the effective VolumeSnapshotLocation per provider for a backup.
///
/// * `specified` — names from `backup.spec.volumeSnapshotLocations`.
/// * `available` — every VSL known to the cluster.
/// * `default_locations` — server-configured default VSL name per provider
///   (`--default-volume-snapshot-locations`).
///
/// Returns a provider→name map on success, or the accumulated validation
/// errors (mirroring `Status.ValidationErrors`).
pub fn validate_and_get_snapshot_locations(
    specified: &[String],
    available: &[VolumeSnapshotLocationInfo],
    default_locations: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, Vec<String>> {
    let mut errors = Vec::new();
    let mut provider_locations: BTreeMap<String, String> = BTreeMap::new();

    // 1. Resolve every explicitly-named location, enforcing one-per-provider.
    for name in specified {
        let Some(found) = available.iter().find(|v| &v.name == name) else {
            errors.push(format!(
                "a VolumeSnapshotLocation CRD for the location {name} with the name specified in the backup spec needs to be created before this snapshot can be executed"
            ));
            continue;
        };
        match provider_locations.get(&found.provider) {
            Some(existing) if existing != name => {
                errors.push(format!(
                    "more than one VolumeSnapshotLocation name specified for provider {}: {existing}; unexpected name was {name}",
                    found.provider
                ));
            }
            _ => {
                provider_locations.insert(found.provider.clone(), name.clone());
            }
        }
    }

    // 2. Fill in providers that were not named explicitly.
    let providers: BTreeSet<&String> = available.iter().map(|v| &v.provider).collect();
    for provider in providers {
        if provider_locations.contains_key(provider) {
            continue;
        }
        let for_provider: Vec<&VolumeSnapshotLocationInfo> =
            available.iter().filter(|v| &v.provider == provider).collect();
        if for_provider.len() == 1 {
            provider_locations.insert(provider.clone(), for_provider[0].name.clone());
        } else if let Some(default_name) = default_locations.get(provider) {
            provider_locations.insert(provider.clone(), default_name.clone());
        } else {
            errors.push(format!(
                "provider {provider} has more than one possible volume snapshot location, and none were specified explicitly or as a default"
            ));
        }
    }

    if errors.is_empty() {
        Ok(provider_locations)
    } else {
        Err(errors)
    }
}

/// Resolve the BackupStorageLocation for a backup that may omit one, falling
/// back to the server default. Mirrors the default-BSL path in
/// `prepareBackupRequest`.
pub fn resolve_default_backup_storage_location(
    specified: Option<&str>,
    server_default: &str,
    available: &[String],
) -> Result<String, String> {
    match specified {
        Some(name) => {
            if available.iter().any(|n| n == name) {
                Ok(name.to_string())
            } else {
                Err(format!(
                    "error getting backup storage location: backupstoragelocation.velero.io \"{name}\" not found"
                ))
            }
        }
        None => {
            if available.iter().any(|n| n == server_default) {
                Ok(server_default.to_string())
            } else {
                Err(format!(
                    "an existing backup storage location was not specified at backup creation time and the server default {server_default} does not exist. Please address this issue (see `velero backup-location -h` for options) and create a new backup. Error: backupstoragelocation.velero.io \"{server_default}\" not found"
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_available_yields_empty_map() {
        let resolved =
            validate_and_get_snapshot_locations(&[], &[], &BTreeMap::new()).unwrap();
        assert!(resolved.is_empty());
    }

    #[test]
    fn explicit_name_unknown_provider_is_independent() {
        let available = vec![
            VolumeSnapshotLocationInfo { name: "aws-1".into(), provider: "aws".into() },
            VolumeSnapshotLocationInfo { name: "gcp-1".into(), provider: "gcp".into() },
        ];
        let resolved =
            validate_and_get_snapshot_locations(&["aws-1".into()], &available, &BTreeMap::new())
                .unwrap();
        // aws explicitly chosen; gcp auto-selected (single).
        assert_eq!(resolved.get("aws"), Some(&"aws-1".to_string()));
        assert_eq!(resolved.get("gcp"), Some(&"gcp-1".to_string()));
    }
}
