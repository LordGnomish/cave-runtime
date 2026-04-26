//! Job controller — runs N pods to completion (with optional parallelism).
//!
//! Upstream: [`pkg/controller/job`]. The full controller implements indexed
//! jobs, backoff per index, suspended jobs, success/failure policies, and
//! the active-deadline timer.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub name: String,
    pub namespace: String,
    pub completions: u32,
    pub parallelism: u32,
    pub backoff_limit: u32,
    pub suspended: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobStatus {
    pub active: u32,
    pub succeeded: u32,
    pub failed: u32,
}

/// True iff `succeeded >= completions`. Mirrors `IsJobFinished` in
/// `pkg/controller/job/util/job_utils.go`.
pub fn is_complete(spec: &JobSpec, status: &JobStatus) -> bool {
    status.succeeded >= spec.completions
}

/// True iff `failed > backoff_limit`. Mirrors `pastBackoffLimitOnFailure` in
/// `pkg/controller/job/job_controller.go`.
pub fn past_backoff(spec: &JobSpec, status: &JobStatus) -> bool {
    status.failed > spec.backoff_limit
}

/// Mirrors `manageJob` in `pkg/controller/job/job_controller.go`. Returns the
/// number of pods to create (clamped by parallelism and remaining completions).
pub fn reconcile(
    spec: &JobSpec,
    status: &JobStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    if spec.suspended {
        // Suspended jobs delete any active pods.
        return Ok(if status.active > 0 {
            Reconcile::Delete(status.active)
        } else {
            Reconcile::NoOp
        });
    }
    if is_complete(spec, status) || past_backoff(spec, status) {
        return Ok(Reconcile::NoOp);
    }
    let remaining = spec.completions.saturating_sub(status.succeeded + status.active);
    let want = remaining.min(spec.parallelism.saturating_sub(status.active));
    if want == 0 {
        Ok(Reconcile::NoOp)
    } else {
        Ok(Reconcile::Create(want))
    }
}

/// Stub: indexed-job per-index status tracking. Not implemented.
pub fn index_status(_spec: &JobSpec) -> Result<Vec<u32>, ControllerError> {
    unimplemented!("Indexed Job — see pkg/controller/job/indexed_job_utils.go")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/job/job_controller.go", "Controller");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn job(parallelism: u32, completions: u32, backoff: u32, suspended: bool) -> JobSpec {
        JobSpec {
            name: "build".into(),
            namespace: "ci".into(),
            completions,
            parallelism,
            backoff_limit: backoff,
            suspended,
        }
    }

    #[test]
    fn launches_up_to_parallelism_when_starting() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "manageJob",
            "tenant-job-launch"
        );
        let s = job(3, 10, 6, false);
        let st = JobStatus::default();
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Create(3));
    }

    #[test]
    fn stops_when_completions_reached() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/util/job_utils.go",
            "IsJobFinished",
            "tenant-job-complete"
        );
        let s = job(3, 5, 6, false);
        let st = JobStatus { active: 0, succeeded: 5, failed: 0 };
        assert!(is_complete(&s, &st));
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::NoOp);
    }

    #[test]
    fn stops_when_past_backoff_limit() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "pastBackoffLimitOnFailure",
            "tenant-job-backoff"
        );
        let s = job(2, 10, 3, false);
        let st = JobStatus { active: 1, succeeded: 0, failed: 4 };
        assert!(past_backoff(&s, &st));
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::NoOp);
    }

    #[test]
    fn suspended_job_deletes_active_pods() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "ensureJobConditionStatus",
            "tenant-job-suspended"
        );
        let s = job(4, 10, 6, true);
        let st = JobStatus { active: 4, succeeded: 0, failed: 0 };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Delete(4));
    }
}
