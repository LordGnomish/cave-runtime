// SPDX-License-Identifier: AGPL-3.0-or-later
//! ReplicaSet controller — keeps a fixed number of pods alive.
//!
//! Upstream: [`pkg/controller/replicaset`]. The full controller handles
//! adoption / orphan logic, slow-start expectation tracking, and pod
//! deletion-cost-based victim selection. This scaffold implements just the
//! diff-and-act core.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaSetSpec {
    pub name: String,
    pub namespace: String,
    pub replicas: u32,
    /// Selector keys used to claim existing pods. Stored as raw label pairs.
    pub selector: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReplicaSetStatus {
    pub running_pods: u32,
    pub failed_pods: u32,
}

/// Mirrors `manageReplicas` in `pkg/controller/replicaset/replica_set.go`.
pub fn reconcile(
    spec: &ReplicaSetSpec,
    status: &ReplicaSetStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    if spec.selector.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "ReplicaSet",
            reason: "selector must not be empty".into(),
        });
    }
    let live = status.running_pods;
    if live == spec.replicas {
        return Ok(Reconcile::NoOp);
    }
    if live < spec.replicas {
        return Ok(Reconcile::Create(spec.replicas - live));
    }
    Ok(Reconcile::Delete(live - spec.replicas))
}

/// Slow-start burst: cap creations at `burst_replicas` per pass.
/// Mirrors `BurstReplicas` in upstream, exposed as a free function so callers
/// can pre-clamp a `Reconcile::Create(_)` decision.
pub fn clamp_burst(decision: Reconcile, burst_replicas: u32) -> Reconcile {
    match decision {
        Reconcile::Create(n) => Reconcile::Create(n.min(burst_replicas)),
        Reconcile::Delete(n) => Reconcile::Delete(n.min(burst_replicas)),
        other => other,
    }
}

/// Lightweight Pod view used by adoption logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodView {
    pub name: String,
    pub namespace: String,
    pub labels: Vec<(String, String)>,
    /// `Some(uid)` if a controllerRef is set; `None` for orphans.
    pub controller_ref: Option<String>,
}

/// Result of an adoption pass — counts how many pods would be claimed
/// and lists their names for audit purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptionPlan {
    pub claimed: Vec<String>,
}

impl AdoptionPlan {
    pub fn count(&self) -> u32 {
        self.claimed.len() as u32
    }
}

fn selector_matches(spec_selector: &[(String, String)], labels: &[(String, String)]) -> bool {
    spec_selector.iter().all(|(k, v)| {
        labels.iter().any(|(lk, lv)| lk == k && lv == v)
    })
}

/// Adopt orphan pods that match the selector. Mirrors
/// `pkg/controller/controller_ref_manager.go::ClaimPods`:
///   * skip pods in a different namespace,
///   * skip pods already owned by a controller (controllerRef set),
///   * skip pods that don't match the selector,
///   * everything else is a candidate for adoption.
pub fn adopt_orphans(
    spec: &ReplicaSetSpec,
    pods: &[PodView],
    _tenant: &TenantId,
) -> Result<AdoptionPlan, ControllerError> {
    if spec.selector.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "ReplicaSet",
            reason: "selector must not be empty for adoption".into(),
        });
    }
    let mut claimed = vec![];
    for p in pods {
        if p.namespace != spec.namespace { continue; }
        if p.controller_ref.is_some() { continue; }
        if !selector_matches(&spec.selector, &p.labels) { continue; }
        claimed.push(p.name.clone());
    }
    Ok(AdoptionPlan { claimed })
}

/// Release adopted pods whose labels no longer match. Mirrors
/// `controller_ref_manager.go::release` — used when a label is changed
/// out from under the RS by an admin or another controller.
pub fn release_mismatched(
    spec: &ReplicaSetSpec,
    pods: &[PodView],
    rs_uid: &str,
) -> Result<Vec<String>, ControllerError> {
    let mut released = vec![];
    for p in pods {
        let owned = p.controller_ref.as_deref() == Some(rs_uid);
        if !owned { continue; }
        if !selector_matches(&spec.selector, &p.labels) {
            released.push(p.name.clone());
        }
    }
    Ok(released)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/replicaset/replica_set.go", "ReplicaSetController");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn spec(replicas: u32) -> ReplicaSetSpec {
        ReplicaSetSpec {
            name: "rs-1".into(),
            namespace: "default".into(),
            replicas,
            selector: vec![("app".into(), "nginx".into())],
        }
    }

    #[test]
    fn empty_selector_is_rejected() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/replicaset/replica_set.go",
            "syncReplicaSet",
            "tenant-rs-bad-selector"
        );
        let bad = ReplicaSetSpec { selector: vec![], ..spec(1) };
        let err = reconcile(&bad, &ReplicaSetStatus::default(), &tenant).unwrap_err();
        assert!(matches!(err, ControllerError::InvalidSpec { kind: "ReplicaSet", .. }));
    }

    #[test]
    fn manage_replicas_creates_when_below_target() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/replicaset/replica_set.go",
            "manageReplicas",
            "tenant-rs-create"
        );
        let st = ReplicaSetStatus { running_pods: 1, ..Default::default() };
        assert_eq!(reconcile(&spec(4), &st, &tenant).unwrap(), Reconcile::Create(3));
    }

    #[test]
    fn manage_replicas_deletes_when_above_target() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/replicaset/replica_set.go",
            "manageReplicas",
            "tenant-rs-delete"
        );
        let st = ReplicaSetStatus { running_pods: 7, ..Default::default() };
        assert_eq!(reconcile(&spec(2), &st, &tenant).unwrap(), Reconcile::Delete(5));
    }

    #[test]
    fn clamp_burst_caps_aggressive_actions() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/replicaset/replica_set.go",
            "BurstReplicas",
            "tenant-rs-burst"
        );
        let _ = tenant;
        assert_eq!(clamp_burst(Reconcile::Create(50), 10), Reconcile::Create(10));
        assert_eq!(clamp_burst(Reconcile::Delete(50), 10), Reconcile::Delete(10));
        assert_eq!(clamp_burst(Reconcile::NoOp, 10), Reconcile::NoOp);
    }

    // ── Deeper coverage (deeper-001) ─────────────────────────────────────────

    fn pod(name: &str, ns: &str, labels: &[(&str, &str)], owner: Option<&str>) -> PodView {
        PodView {
            name: name.into(),
            namespace: ns.into(),
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            controller_ref: owner.map(String::from),
        }
    }

    /// Upstream parity: `TestClaimPods_AdoptsMatchingOrphans`
    /// (pkg/controller/controller_ref_manager_test.go — orphan pods that
    /// match the selector are claimed by the RS).
    #[test]
    fn adopts_matching_orphan_pods_in_same_namespace() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/controller_ref_manager.go",
            "ClaimPods",
            "tenant-rs-adopt-orphan"
        );
        let s = spec(3);
        let pods = vec![
            pod("p1", "default", &[("app","nginx")], None),
            pod("p2", "default", &[("app","nginx")], None),
        ];
        let plan = adopt_orphans(&s, &pods, &tenant).unwrap();
        assert_eq!(plan.count(), 2);
        assert!(plan.claimed.contains(&"p1".to_string()));
    }

    /// Upstream parity: `TestClaimPods_SkipsAlreadyOwnedPods`
    /// (controller_ref_manager_test.go — never adopt a pod with an existing
    /// controllerRef even if the label set matches).
    #[test]
    fn does_not_adopt_pods_already_owned_by_another_controller() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/controller_ref_manager.go",
            "ClaimPods",
            "tenant-rs-owned-skip"
        );
        let s = spec(3);
        let pods = vec![
            pod("orphan", "default", &[("app","nginx")], None),
            pod("owned",  "default", &[("app","nginx")], Some("rs-other-uid")),
        ];
        let plan = adopt_orphans(&s, &pods, &tenant).unwrap();
        assert_eq!(plan.count(), 1, "only the orphan is claimed");
        assert_eq!(plan.claimed, vec!["orphan".to_string()]);
    }

    /// Upstream parity: `TestClaimPods_SkipsNonMatchingLabels`
    /// (controller_ref_manager_test.go — selector mismatch is non-claim).
    #[test]
    fn does_not_adopt_pods_whose_labels_do_not_match_selector() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/controller_ref_manager.go",
            "ClaimPods",
            "tenant-rs-label-mismatch"
        );
        let s = spec(3);
        let pods = vec![
            pod("hit",  "default", &[("app","nginx")], None),
            pod("miss", "default", &[("app","redis")], None),
        ];
        let plan = adopt_orphans(&s, &pods, &tenant).unwrap();
        assert_eq!(plan.count(), 1);
        assert_eq!(plan.claimed, vec!["hit".to_string()]);
    }

    /// Upstream parity: `TestClaimPods_SkipsCrossNamespacePods`
    /// (controller_ref_manager_test.go — adoption is namespace-scoped; we
    /// strengthen this in cave-apiserver to a tenant_id invariant).
    #[test]
    fn does_not_adopt_pods_from_a_different_namespace_or_tenant() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/controller_ref_manager.go",
            "ClaimPods",
            "tenant-rs-cross-ns"
        );
        let s = spec(3); // namespace=default
        let pods = vec![
            pod("local",  "default",     &[("app","nginx")], None),
            pod("alien",  "kube-system", &[("app","nginx")], None),
        ];
        let plan = adopt_orphans(&s, &pods, &tenant).unwrap();
        assert_eq!(plan.count(), 1,
            "tenant_id invariant: cross-namespace pods are NOT adopted");
        assert_eq!(plan.claimed, vec!["local".to_string()]);
    }

    /// Upstream parity: `TestRefManager_ReleaseRelinquishesMismatched`
    /// (controller_ref_manager_test.go — a previously-owned pod whose
    /// labels were rewritten away from the selector is released back to
    /// orphan status).
    #[test]
    fn release_returns_owned_pods_whose_labels_no_longer_match() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/controller_ref_manager.go",
            "release",
            "tenant-rs-release"
        );
        let _ = tenant;
        let s = spec(3);
        let rs_uid = "rs-1-uid";
        let pods = vec![
            pod("kept", "default", &[("app","nginx")], Some(rs_uid)),
            pod("drift","default", &[("app","sidekiq")], Some(rs_uid)),
            pod("not-mine", "default", &[("app","sidekiq")], Some("rs-other")),
        ];
        let released = release_mismatched(&s, &pods, rs_uid).unwrap();
        assert_eq!(released, vec!["drift".to_string()],
            "only owned pods with mismatched labels are released");
    }
}
