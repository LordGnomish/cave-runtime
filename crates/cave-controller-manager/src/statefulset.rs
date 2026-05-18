// SPDX-License-Identifier: AGPL-3.0-or-later
//! StatefulSet controller — ordered, identity-stable pod management.
//!
//! Upstream: [`pkg/controller/statefulset`]. Identity is `<name>-<ordinal>`;
//! pods are created in ascending ordinal order, deleted in descending order,
//! and PVCs are retained or deleted per the
//! `persistentVolumeClaimRetentionPolicy` block in upstream v1.36.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodManagementPolicy {
    OrderedReady,
    Parallel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatefulSetSpec {
    pub name: String,
    pub namespace: String,
    pub replicas: u32,
    pub policy: PodManagementPolicy,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatefulSetStatus {
    pub current_replicas: u32,
    pub ready_replicas: u32,
}

/// Pod identity for ordinal `i` is `<name>-<i>`. Mirrors `getPodName` in
/// `pkg/controller/statefulset/stateful_set_utils.go`.
pub fn pod_identity(spec: &StatefulSetSpec, ordinal: u32) -> String {
    format!("{}-{}", spec.name, ordinal)
}

/// Mirrors `updateStatefulSet` in upstream — the Ordered policy may only act
/// on one pod per pass, the Parallel policy can fan out.
pub fn reconcile(
    spec: &StatefulSetSpec,
    status: &StatefulSetStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    if status.current_replicas == spec.replicas {
        return Ok(Reconcile::NoOp);
    }
    let diff = spec.replicas as i64 - status.current_replicas as i64;
    let abs = diff.unsigned_abs() as u32;
    let step = match spec.policy {
        PodManagementPolicy::OrderedReady => 1,
        PodManagementPolicy::Parallel => abs,
    };
    if diff > 0 {
        Ok(Reconcile::Create(step))
    } else {
        Ok(Reconcile::Delete(step))
    }
}

/// PVC retention policy. Mirrors `apps/v1.StatefulSetPersistentVolumeClaimRetentionPolicy`
/// from upstream v1.36.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PvcRetentionPolicy {
    /// Default: keep PVCs after pod or set deletion.
    Retain,
    /// Delete the PVC when the owning resource (Pod or StatefulSet) is deleted.
    Delete,
}

impl Default for PvcRetentionPolicy {
    fn default() -> Self { Self::Retain }
}

/// Volume claim template — name plus the retention policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeClaimTemplate {
    pub name: String,
    pub when_deleted: PvcRetentionPolicy,
    pub when_scaled: PvcRetentionPolicy,
}

/// Compute the canonical PVC name for `(template, ordinal)`. Mirrors
/// `pkg/controller/statefulset/stateful_set_utils.go::getPersistentVolumeClaimName`:
///   `<template-name>-<sts-name>-<ordinal>`
pub fn pvc_name_for(spec: &StatefulSetSpec, template: &VolumeClaimTemplate, ordinal: u32) -> String {
    format!("{}-{}-{}", template.name, spec.name, ordinal)
}

/// Plan PVCs to delete when the StatefulSet is scaled down from `was` to
/// `now`. Honours `when_scaled`. Mirrors
/// `pkg/controller/statefulset/stateful_set_control.go::shouldDeletePvc`.
pub fn pvcs_to_delete_on_scale_down(
    spec: &StatefulSetSpec,
    template: &VolumeClaimTemplate,
    was: u32,
    now: u32,
) -> Vec<String> {
    if now >= was || template.when_scaled == PvcRetentionPolicy::Retain {
        return vec![];
    }
    (now..was).map(|i| pvc_name_for(spec, template, i)).collect()
}

/// Plan PVCs to delete when the entire StatefulSet is deleted. Honours
/// `when_deleted`. Mirrors
/// `pkg/controller/statefulset/stateful_set_control.go::deleteOwnerRefForPvcs`.
pub fn pvcs_to_delete_on_set_deletion(
    spec: &StatefulSetSpec,
    template: &VolumeClaimTemplate,
    current: u32,
) -> Vec<String> {
    if template.when_deleted == PvcRetentionPolicy::Retain {
        return vec![];
    }
    (0..current).map(|i| pvc_name_for(spec, template, i)).collect()
}

/// Ordinal range. With KEP-3335 (`spec.ordinals.start`) the lower bound
/// can be non-zero; baseline implementation here treats start=0.
/// Mirrors `pkg/controller/statefulset/stateful_set_utils.go::getStartOrdinal`.
pub fn ordinal_range(spec: &StatefulSetSpec) -> std::ops::Range<u32> {
    0u32..spec.replicas
}

/// Pod creation order for the OrderedReady policy: ascending ordinals.
/// Mirrors `pkg/controller/statefulset/stateful_set_control.go::canCreate`.
pub fn ordered_creation_sequence(spec: &StatefulSetSpec) -> Vec<String> {
    ordinal_range(spec).map(|i| pod_identity(spec, i)).collect()
}

/// Pod deletion order for the OrderedReady policy: descending ordinals.
/// Mirrors `pkg/controller/statefulset/stateful_set_control.go::burst`.
pub fn ordered_deletion_sequence(spec: &StatefulSetSpec, current: u32) -> Vec<String> {
    let mut out: Vec<String> = (0..current).map(|i| pod_identity(spec, i)).collect();
    out.reverse();
    out
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/statefulset/stateful_set.go", "StatefulSetController");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn spec(replicas: u32, policy: PodManagementPolicy) -> StatefulSetSpec {
        StatefulSetSpec { name: "db".into(), namespace: "ns".into(), replicas, policy }
    }

    #[test]
    fn pod_identity_is_name_dash_ordinal() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_utils.go",
            "getPodName",
            "tenant-sts-identity"
        );
        let _ = tenant;
        let s = spec(3, PodManagementPolicy::OrderedReady);
        assert_eq!(pod_identity(&s, 0), "db-0");
        assert_eq!(pod_identity(&s, 2), "db-2");
    }

    #[test]
    fn ordered_policy_acts_on_one_pod_per_pass() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "updateStatefulSet",
            "tenant-sts-ordered"
        );
        let s = spec(5, PodManagementPolicy::OrderedReady);
        let st = StatefulSetStatus { current_replicas: 1, ..Default::default() };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Create(1));
    }

    #[test]
    fn parallel_policy_fans_out() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "updateStatefulSet",
            "tenant-sts-parallel"
        );
        let s = spec(5, PodManagementPolicy::Parallel);
        let st = StatefulSetStatus { current_replicas: 1, ..Default::default() };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Create(4));
    }

    #[test]
    fn scale_down_emits_delete() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "updateStatefulSet",
            "tenant-sts-scale-down"
        );
        let s = spec(2, PodManagementPolicy::OrderedReady);
        let st = StatefulSetStatus { current_replicas: 5, ..Default::default() };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Delete(1));
    }

    // ── Deeper coverage (deeper-001) ─────────────────────────────────────────

    fn vct(name: &str, when_deleted: PvcRetentionPolicy, when_scaled: PvcRetentionPolicy) -> VolumeClaimTemplate {
        VolumeClaimTemplate {
            name: name.into(), when_deleted, when_scaled,
        }
    }

    /// Upstream parity: `TestStatefulSet_OrderedCreationSequence`
    /// (stateful_set_control_test.go — OrderedReady creates pods 0..N
    /// in ascending ordinal order).
    #[test]
    fn ordered_policy_creation_sequence_is_ascending() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "canCreate",
            "tenant-sts-create-order"
        );
        let _ = tenant;
        let s = spec(4, PodManagementPolicy::OrderedReady);
        let seq = ordered_creation_sequence(&s);
        assert_eq!(seq, vec!["db-0", "db-1", "db-2", "db-3"]);
    }

    /// Upstream parity: `TestStatefulSet_OrderedDeletionSequence`
    /// (stateful_set_control_test.go — OrderedReady deletes pods in
    /// descending ordinal order).
    #[test]
    fn ordered_policy_deletion_sequence_is_descending() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "burst",
            "tenant-sts-delete-order"
        );
        let _ = tenant;
        let s = spec(3, PodManagementPolicy::OrderedReady);
        let seq = ordered_deletion_sequence(&s, 3);
        assert_eq!(seq, vec!["db-2", "db-1", "db-0"]);
    }

    /// Upstream parity: `TestStatefulSet_PvcNameFormat`
    /// (stateful_set_utils_test.go::TestGetPersistentVolumeClaimName —
    /// canonical name is `<template>-<sts>-<ordinal>`).
    #[test]
    fn pvc_name_uses_template_dash_sts_dash_ordinal_format() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_utils.go",
            "getPersistentVolumeClaimName",
            "tenant-sts-pvc-name"
        );
        let _ = tenant;
        let s = spec(3, PodManagementPolicy::OrderedReady);
        let t = vct("data", PvcRetentionPolicy::Retain, PvcRetentionPolicy::Retain);
        assert_eq!(pvc_name_for(&s, &t, 0), "data-db-0");
        assert_eq!(pvc_name_for(&s, &t, 2), "data-db-2");
    }

    /// Upstream parity: `TestStatefulSet_PvcRetainOnScaleDown`
    /// (stateful_set_control_test.go — `whenScaled: Retain` keeps PVCs
    /// alive even after the matching pods are deleted).
    #[test]
    fn pvc_retention_retain_keeps_pvcs_through_scale_down() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "shouldDeletePvc",
            "tenant-sts-pvc-retain"
        );
        let _ = tenant;
        let s = spec(3, PodManagementPolicy::OrderedReady);
        let t = vct("data", PvcRetentionPolicy::Retain, PvcRetentionPolicy::Retain);
        let to_delete = pvcs_to_delete_on_scale_down(&s, &t, /*was=*/ 5, /*now=*/ 3);
        assert!(to_delete.is_empty(),
            "Retain policy keeps PVCs even after pod scale-down");
    }

    /// Upstream parity: `TestStatefulSet_PvcDeleteOnScaleDown`
    /// (stateful_set_control_test.go — `whenScaled: Delete` removes PVCs
    /// for ordinals dropped during scale-down).
    #[test]
    fn pvc_retention_delete_on_scale_down_removes_dropped_ordinals() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "shouldDeletePvc",
            "tenant-sts-pvc-delete-scale"
        );
        let _ = tenant;
        let s = spec(2, PodManagementPolicy::OrderedReady);
        let t = vct("data", PvcRetentionPolicy::Retain, PvcRetentionPolicy::Delete);
        let to_delete = pvcs_to_delete_on_scale_down(&s, &t, /*was=*/ 5, /*now=*/ 2);
        assert_eq!(to_delete, vec![
            "data-db-2".to_string(),
            "data-db-3".to_string(),
            "data-db-4".to_string(),
        ]);
    }

    /// Upstream parity: `TestStatefulSet_PvcDeleteOnSetDeletion`
    /// (stateful_set_control_test.go — `whenDeleted: Delete` removes
    /// every PVC when the StatefulSet itself is deleted).
    #[test]
    fn pvc_retention_delete_on_set_deletion_clears_every_pvc() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/statefulset/stateful_set_control.go",
            "deleteOwnerRefForPvcs",
            "tenant-sts-pvc-delete-set"
        );
        let _ = tenant;
        let s = spec(3, PodManagementPolicy::OrderedReady);
        let t = vct("data", PvcRetentionPolicy::Delete, PvcRetentionPolicy::Retain);
        let to_delete = pvcs_to_delete_on_set_deletion(&s, &t, /*current=*/ 3);
        assert_eq!(to_delete, vec![
            "data-db-0".to_string(),
            "data-db-1".to_string(),
            "data-db-2".to_string(),
        ]);
    }
}
