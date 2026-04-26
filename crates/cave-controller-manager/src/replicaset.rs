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

/// Stub: adopt orphan pods that match the selector. Not implemented.
pub fn adopt_orphans(_spec: &ReplicaSetSpec) -> Result<u32, ControllerError> {
    unimplemented!("ReplicaSet orphan adoption — see ClaimPods in pkg/controller/controller_ref_manager.go")
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
}
