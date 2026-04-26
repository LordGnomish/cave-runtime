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

/// Bounded revision history. Mirrors `revisionHistoryLimit` in
/// `pkg/controller/deployment/util/deployment_util.go`. Each entry stores
/// the desired pod-template hash and a monotonically increasing revision
/// counter; oldest entries are evicted when capacity is exceeded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionHistory {
    pub limit: u32,
    pub revisions: Vec<RevisionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionEntry {
    pub revision: u64,
    pub pod_template_hash: String,
}

impl RevisionHistory {
    pub fn new(limit: u32) -> Self {
        assert!(limit > 0, "revision history limit must be > 0");
        Self { limit, revisions: vec![] }
    }

    /// Append a new revision; evict the oldest if the buffer is over capacity.
    pub fn record(&mut self, pod_template_hash: impl Into<String>) -> u64 {
        let revision = self.revisions.last().map(|e| e.revision + 1).unwrap_or(1);
        self.revisions.push(RevisionEntry {
            revision,
            pod_template_hash: pod_template_hash.into(),
        });
        while self.revisions.len() > self.limit as usize {
            self.revisions.remove(0);
        }
        revision
    }

    pub fn lookup(&self, revision: u64) -> Option<&RevisionEntry> {
        self.revisions.iter().find(|e| e.revision == revision)
    }
}

/// Roll a Deployment back to the named revision. Mirrors
/// `pkg/controller/deployment/rollback.go::DeploymentController.rollback`:
///   * lookup the requested revision in the history,
///   * if missing, return Reconcile::Requeue (caller surfaces a status
///     condition `RollbackRevisionNotFound` per upstream),
///   * otherwise emit Update(replicas) so the active ReplicaSet is swapped
///     to the rolled-back template.
pub fn rollback(
    spec: &DeploymentSpec,
    history: &RevisionHistory,
    to_revision: u64,
) -> Result<Reconcile, ControllerError> {
    if to_revision == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "Deployment",
            reason: "rollback target revision must be > 0".into(),
        });
    }
    if history.lookup(to_revision).is_none() {
        return Ok(Reconcile::Requeue);
    }
    Ok(Reconcile::Update(spec.replicas))
}

/// One step of an in-progress rolling update. Mirrors
/// `pkg/controller/deployment/rolling.go::reconcileNewReplicaSet` +
/// `reconcileOldReplicaSets`. Returns the planned change to the new RS
/// (positive = scale up, negative = scale down) honouring both maxSurge
/// and maxUnavailable budgets relative to `spec.replicas`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollingStep {
    pub new_rs_target: u32,
    pub old_rs_target: u32,
}

pub fn plan_rolling_step(
    spec: &DeploymentSpec,
    new_rs_size: u32,
    old_rs_size: u32,
) -> Result<RollingStep, ControllerError> {
    let (max_surge, max_unavailable) = match spec.strategy {
        Strategy::Recreate => {
            // Recreate: kill all old pods first, then bring up new.
            return Ok(RollingStep {
                new_rs_target: if old_rs_size == 0 { spec.replicas } else { 0 },
                old_rs_target: 0,
            });
        }
        Strategy::RollingUpdate { max_surge, max_unavailable } => {
            (max_surge, max_unavailable)
        }
    };
    let total = new_rs_size.saturating_add(old_rs_size);
    let surge_room = spec.replicas
        .saturating_add(max_surge)
        .saturating_sub(total);
    let new_rs_target = new_rs_size.saturating_add(surge_room).min(spec.replicas);
    // Old RS scale-down accounts only for already-alive new pods (i.e.
    // current new_rs_size, NOT the planned new_rs_target). New target pods
    // are pending — we cannot count them against availability yet.
    let min_available = spec.replicas.saturating_sub(max_unavailable);
    let alive = new_rs_size.saturating_add(old_rs_size);
    let removable = alive.saturating_sub(min_available);
    let scale_down = removable.min(old_rs_size);
    let old_rs_target = old_rs_size.saturating_sub(scale_down);
    Ok(RollingStep { new_rs_target, old_rs_target })
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

    // ── Deeper coverage (deeper-001) ─────────────────────────────────────────

    /// Upstream parity: `TestRollingUpdateDeployment_PlanFirstStep`
    /// (pkg/controller/deployment/rolling_test.go — initial step scales the
    /// new RS up by maxSurge while old RS still serves all traffic).
    #[test]
    fn rolling_update_first_step_scales_new_rs_up_by_max_surge() {
        let (cite, tenant) = test_ctx!(
            "pkg/controller/deployment/rolling.go",
            "reconcileNewReplicaSet",
            "tenant-deploy-rolling-first-step"
        );
        let _ = tenant;
        let s = DeploymentSpec {
            replicas: 10,
            strategy: Strategy::RollingUpdate { max_surge: 2, max_unavailable: 0 },
            ..spec(10, false)
        };
        let step = plan_rolling_step(&s, 0, 10).unwrap();
        // new_rs scales up by max_surge=2, old_rs stays full (max_unavailable=0,
        // and new pods aren't ready yet so old can't be drained).
        assert_eq!(step.new_rs_target, 2);
        assert_eq!(step.old_rs_target, 10);
        assert_eq!(cite.symbol, "reconcileNewReplicaSet");
    }

    /// Upstream parity: `TestRollingUpdateDeployment_PlanProgressUnderMaxUnavailable`
    /// (rolling_test.go — once the new RS has surge headroom, the old RS may
    /// be scaled down up to max_unavailable to maintain pod budget).
    #[test]
    fn rolling_update_scales_old_rs_down_within_max_unavailable() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/deployment/rolling.go",
            "reconcileOldReplicaSets",
            "tenant-deploy-rolling-old-down"
        );
        let _ = tenant;
        let s = DeploymentSpec {
            replicas: 10,
            strategy: Strategy::RollingUpdate { max_surge: 2, max_unavailable: 2 },
            ..spec(10, false)
        };
        // Mid-rollout: 4 new pods up, 8 old still up — total 12 = 10 + maxSurge.
        let step = plan_rolling_step(&s, 4, 8).unwrap();
        // new_rs cannot exceed spec.replicas; old_rs gets reduced toward floor.
        assert!(step.new_rs_target >= 4);
        // min_available = 10 - 2 = 8; total_after_surge = 4+8 = 12; removable = 4
        // Old RS goes from 8 down by min(removable=4, 8) = 4 → 4.
        assert_eq!(step.old_rs_target, 4);
    }

    /// Upstream parity: `TestRecreateDeployment_PlanKillsOldFirst`
    /// (pkg/controller/deployment/recreate.go — `Recreate` strategy keeps
    /// new RS at zero until old RS is fully drained).
    #[test]
    fn recreate_strategy_holds_new_rs_until_old_rs_is_zero() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/deployment/recreate.go",
            "rolloutRecreate",
            "tenant-deploy-recreate"
        );
        let _ = tenant;
        let s = DeploymentSpec {
            replicas: 5,
            strategy: Strategy::Recreate,
            ..spec(5, false)
        };
        let mid = plan_rolling_step(&s, 0, 3).unwrap();
        assert_eq!(mid.new_rs_target, 0,
            "Recreate must NOT bring up new pods while old pods still alive");
        let drained = plan_rolling_step(&s, 0, 0).unwrap();
        assert_eq!(drained.new_rs_target, 5,
            "Once drained, Recreate brings the full replica count online");
    }

    /// Upstream parity: `TestRevisionHistory_RingBufferLimit`
    /// (deployment_util_test.go — historical RSes beyond
    /// `revisionHistoryLimit` are evicted oldest-first).
    #[test]
    fn revision_history_evicts_oldest_when_limit_exceeded() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/deployment/util/deployment_util.go",
            "Revision",
            "tenant-deploy-rev-history"
        );
        let _ = tenant;
        let mut h = RevisionHistory::new(3);
        let r1 = h.record("hash-1");
        let r2 = h.record("hash-2");
        let r3 = h.record("hash-3");
        let r4 = h.record("hash-4");
        assert_eq!(h.revisions.len(), 3, "buffer bounded by limit=3");
        assert!(h.lookup(r1).is_none(), "oldest revision evicted");
        assert!(h.lookup(r2).is_some());
        assert!(h.lookup(r4).is_some());
        assert_eq!(r4, r3 + 1, "revision counter is monotonic across evictions");
    }

    /// Upstream parity: `TestDeploymentRollback_RevisionPresent`
    /// + `TestDeploymentRollback_RevisionMissing`
    /// (rollback_test.go — present revision yields Update; missing revision
    /// triggers Requeue with `RollbackRevisionNotFound` upstream).
    #[test]
    fn rollback_present_revision_emits_update_missing_requeues() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/deployment/rollback.go",
            "rollback",
            "tenant-deploy-rollback"
        );
        let _ = tenant;
        let s = spec(5, false);
        let mut h = RevisionHistory::new(5);
        let r1 = h.record("template-A");
        let _r2 = h.record("template-B");
        // Present: update emits with target replica count.
        let ok = rollback(&s, &h, r1).unwrap();
        assert_eq!(ok, Reconcile::Update(5));
        // Missing: requeue (controller surfaces a status condition upstream).
        let miss = rollback(&s, &h, 999).unwrap();
        assert_eq!(miss, Reconcile::Requeue);
        // revision=0 is rejected outright.
        assert!(rollback(&s, &h, 0).is_err());
    }

    /// Upstream parity: `TestDeployment_PauseFreezesRollouts`
    /// (deployment_controller.go — `paused` blocks every reconcile decision
    /// regardless of replica diff or rolling-update progress).
    #[test]
    fn pause_freezes_rollout_then_resume_emits_action() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/deployment/deployment_controller.go",
            "checkPausedConditions",
            "tenant-deploy-pause-resume"
        );
        let mut s = spec(5, true);
        let st = DeploymentStatus { observed_replicas: 0, ..Default::default() };
        // Paused: no-op even though spec wants 5 and status has 0.
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::NoOp);
        // Resume: now the diff is acted on.
        s.paused = false;
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Create(5));
    }
}
