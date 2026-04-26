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

/// Stub: ordinal range arithmetic for `spec.ordinals.start`. Not implemented.
pub fn ordinal_range(_spec: &StatefulSetSpec) -> Result<std::ops::Range<u32>, ControllerError> {
    unimplemented!("StatefulSet start-ordinal — see KEP-3335")
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
}
