// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Backup-request preparation engine.
//!
//! Pure port of the defaulting and validation logic in Velero's
//! `pkg/controller/backup_controller.go` `prepareBackupRequest` (Apache-2.0).
//! The Kubernetes controller-runtime reconcile loop itself is owned by
//! cave-controller-manager; this module captures only the deterministic,
//! runtime-free business logic — applying spec defaults and collecting the
//! `Status.ValidationErrors` list — so it is honestly line-portable.

use crate::includes_excludes::validate_includes_excludes;

/// Server-side default durations injected into a backup spec when the user
/// leaves a field at zero. Mirrors `backupReconciler.defaultBackupTTL` /
/// `defaultCSISnapshotTimeout` / `defaultItemOperationTimeout`.
#[derive(Debug, Clone)]
pub struct BackupDefaults {
    pub ttl_hours: u64,
    pub csi_snapshot_timeout_minutes: u64,
    pub item_operation_timeout_minutes: u64,
}

/// The subset of a Velero `BackupSpec` that participates in defaulting and
/// validation. Fields mirror the upstream spec field names.
#[derive(Debug, Clone, Default)]
pub struct BackupRequestSpec {
    pub ttl_hours: u64,
    pub csi_snapshot_timeout_minutes: u64,
    pub item_operation_timeout_minutes: u64,
    pub included_namespaces: Vec<String>,
    pub excluded_namespaces: Vec<String>,
    // Old (v1) resource filters.
    pub included_resources: Vec<String>,
    pub excluded_resources: Vec<String>,
    pub include_cluster_resources: Option<bool>,
    // New scoped resource filters.
    pub included_cluster_scoped_resources: Vec<String>,
    pub excluded_cluster_scoped_resources: Vec<String>,
    pub included_namespace_scoped_resources: Vec<String>,
    pub excluded_namespace_scoped_resources: Vec<String>,
    // Label selection.
    pub label_selector: Option<String>,
    pub or_label_selectors: Vec<String>,
    // Whether a resource-modifier / include-exclude policy is attached.
    pub has_resource_policies: bool,
}

impl BackupRequestSpec {
    /// True when any of the old (v1) resource filter parameters are set.
    fn uses_old_filters(&self) -> bool {
        !self.included_resources.is_empty()
            || !self.excluded_resources.is_empty()
            || self.include_cluster_resources.is_some()
    }

    /// True when any of the new scoped resource filter parameters are set.
    fn uses_new_filters(&self) -> bool {
        !self.included_cluster_scoped_resources.is_empty()
            || !self.excluded_cluster_scoped_resources.is_empty()
            || !self.included_namespace_scoped_resources.is_empty()
            || !self.excluded_namespace_scoped_resources.is_empty()
    }
}

/// Apply server defaults to `spec` in place and return the list of validation
/// errors (mirrors appending to `request.Status.ValidationErrors`). An empty
/// vec means the request is valid.
pub fn prepare_backup_request(spec: &mut BackupRequestSpec, defaults: &BackupDefaults) -> Vec<String> {
    // ── Defaulting ──────────────────────────────────────────────────────────
    if spec.ttl_hours == 0 {
        spec.ttl_hours = defaults.ttl_hours;
    }
    if spec.csi_snapshot_timeout_minutes == 0 {
        spec.csi_snapshot_timeout_minutes = defaults.csi_snapshot_timeout_minutes;
    }
    if spec.item_operation_timeout_minutes == 0 {
        spec.item_operation_timeout_minutes = defaults.item_operation_timeout_minutes;
    }
    if spec.included_namespaces.is_empty() {
        spec.included_namespaces = vec!["*".to_string()];
    }

    // ── Validation ──────────────────────────────────────────────────────────
    let mut errs = Vec::new();

    let old = spec.uses_old_filters();
    let new = spec.uses_new_filters();

    // Old and new filter families are mutually exclusive.
    if old && new {
        errs.push(
            "include-resources, exclude-resources and include-cluster-resources are old filter parameters.\n\
include-cluster-scoped-resources, exclude-cluster-scoped-resources, include-namespace-scoped-resources and exclude-namespace-scoped-resources are new filter parameters.\n\
They cannot be used together"
                .to_string(),
        );
    }

    // Old filters cannot be combined with include-exclude resource policies.
    if old && spec.has_resource_policies {
        errs.push(
            "include-resources, exclude-resources and include-cluster-resources are old filter parameters.\n\
They cannot be used with include-exclude policies."
                .to_string(),
        );
    }

    // Resource list validation.
    if new {
        let cluster =
            validate_includes_excludes(&spec.included_cluster_scoped_resources, &spec.excluded_cluster_scoped_resources);
        if !cluster.is_empty() {
            errs.push(format!(
                "Invalid cluster-scoped included/excluded resource lists: {}",
                cluster.join("; ")
            ));
        }
        let namespaced = validate_includes_excludes(
            &spec.included_namespace_scoped_resources,
            &spec.excluded_namespace_scoped_resources,
        );
        if !namespaced.is_empty() {
            errs.push(format!(
                "Invalid namespace-scoped included/excluded resource lists: {}",
                namespaced.join("; ")
            ));
        }
    } else {
        let res = validate_includes_excludes(&spec.included_resources, &spec.excluded_resources);
        if !res.is_empty() {
            errs.push(format!("Invalid included/excluded resource lists: {}", res.join("; ")));
        }
    }

    // Namespace list validation.
    let ns = validate_includes_excludes(&spec.included_namespaces, &spec.excluded_namespaces);
    if !ns.is_empty() {
        errs.push(format!("Invalid included/excluded namespace lists: {}", ns.join("; ")));
    }

    // Label selector mutual exclusion.
    if spec.label_selector.is_some() && !spec.or_label_selectors.is_empty() {
        errs.push(
            "encountered labelSelector as well as orLabelSelectors in backup spec, only one can be specified"
                .to_string(),
        );
    }

    errs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> BackupDefaults {
        BackupDefaults {
            ttl_hours: 720,
            csi_snapshot_timeout_minutes: 10,
            item_operation_timeout_minutes: 240,
        }
    }

    #[test]
    fn default_namespace_is_wildcard() {
        let mut spec = BackupRequestSpec::default();
        let errs = prepare_backup_request(&mut spec, &defaults());
        assert!(errs.is_empty());
        assert_eq!(spec.included_namespaces, vec!["*".to_string()]);
    }

    #[test]
    fn excludes_wildcard_namespace_is_invalid() {
        let mut spec = BackupRequestSpec::default();
        spec.included_namespaces = vec!["prod".into()];
        spec.excluded_namespaces = vec!["*".into()];
        let errs = prepare_backup_request(&mut spec, &defaults());
        assert!(errs.iter().any(|e| e.starts_with("Invalid included/excluded namespace lists: ")));
    }
}
