//! Deployment controller — tracks `apps/v1.Deployment` and its owned
//! ReplicaSets.
//!
//! Upstream: [`pkg/controller/deployment`]. The full controller does
//! rolling-update planning (max-surge / max-unavailable), rollback, pause,
//! and ReplicaSet GC. This scaffold implements only replica diffing and
//! defers strategy bodies to [`unimplemented!`].

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

/// `apps/v1.DeploymentStrategy.Type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Strategy {
    Recreate,
    RollingUpdate { max_surge: u32, max_unavailable: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentSpec {
    pub name: String,
    pub namespace: String,
    pub replicas: u32,
    pub strategy: Strategy,
    pub paused: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeploymentStatus {
    pub observed_replicas: u32,
    pub ready_replicas: u32,
    pub updated_replicas: u32,
    pub available_replicas: u32,
}

/// One reconciliation pass. Returns the decision the controller would issue
/// against the API server.
///
/// Mirrors `syncDeployment` in `pkg/controller/deployment/deployment_controller.go`.
pub fn reconcile(
    spec: &DeploymentSpec,
    status: &DeploymentStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    if spec.paused {
        return Ok(Reconcile::NoOp);
    }
    if status.observed_replicas == spec.replicas {
        return Ok(Reconcile::NoOp);
    }
    if status.observed_replicas < spec.replicas {
        return Ok(Reconcile::Create(spec.replicas - status.observed_replicas));
    }
    Ok(Reconcile::Delete(status.observed_replicas - spec.replicas))
}

/// Compute the maximum number of pods that may exist concurrently during a
/// rolling update. Mirrors `MaxSurge` in `pkg/controller/deployment/util/deployment_util.go`.
pub fn max_pods_during_surge(spec: &DeploymentSpec) -> u32 {
    match spec.strategy {
        Strategy::Recreate => spec.replicas,
        Strategy::RollingUpdate { max_surge, .. } => spec.replicas.saturating_add(max_surge),
    }
}

/// Stub: rollback to a previous revision. Not implemented in this scaffold.
pub fn rollback(_spec: &DeploymentSpec, _to_revision: u64) -> Result<Reconcile, ControllerError> {
    unimplemented!("Deployment rollback — see pkg/controller/deployment/rollback.go")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/deployment/deployment_controller.go",
    "DeploymentController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn spec(replicas: u32, paused: bool) -> DeploymentSpec {
        DeploymentSpec {
            name: "web".into(),
            namespace: "default".into(),
            replicas,
            strategy: Strategy::RollingUpdate { max_surge: 1, max_unavailable: 0 },
            paused,
        }
    }

    #[test]
    fn scale_up_creates_missing_replicas() {
        let (cite, tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "scale",
            "tenant-deploy-scale-up"
        );
        let s = spec(5, false);
        let st = DeploymentStatus { observed_replicas: 2, ..Default::default() };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Create(3));
        assert_eq!(cite.symbol, "scale");
    }

    #[test]
    fn scale_down_deletes_excess_replicas() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "scaleDownOldReplicaSetsForRollingUpdate",
            "tenant-deploy-scale-down"
        );
        let s = spec(2, false);
        let st = DeploymentStatus { observed_replicas: 5, ..Default::default() };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Delete(3));
    }

    #[test]
    fn paused_deployment_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/deployment/sync.go",
            "checkPausedConditions",
            "tenant-deploy-paused"
        );
        let s = spec(10, true);
        let st = DeploymentStatus::default();
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::NoOp);
    }

    #[test]
    fn surge_budget_includes_max_surge() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/deployment/util/deployment_util.go",
            "MaxSurge",
            "tenant-deploy-surge"
        );
        let _ = tenant;
        let s = spec(4, false);
        assert_eq!(max_pods_during_surge(&s), 5);
        let recreate = DeploymentSpec { strategy: Strategy::Recreate, ..s };
        assert_eq!(max_pods_during_surge(&recreate), 4);
    }
}
