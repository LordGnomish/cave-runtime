// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Job controller — runs N pods to completion (with optional parallelism).
//!
//! Upstream: [`pkg/controller/job`]. The full controller implements indexed
//! jobs, backoff per index, suspended jobs, success/failure policies, and
//! the active-deadline timer.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSpec {
    pub name: String,
    pub namespace: String,
    pub completions: u32,
    pub parallelism: u32,
    pub backoff_limit: u32,
    pub suspended: bool,
    /// Mirrors `Job.Spec.ActiveDeadlineSeconds`. When set, the controller
    /// drains active pods and emits `JobReasonDeadlineExceeded` once
    /// `now - status.start_time >= active_deadline_seconds`. `None`
    /// means no deadline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_deadline_seconds: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobStatus {
    pub active: u32,
    pub succeeded: u32,
    pub failed: u32,
    /// Mirrors `Job.Status.StartTime`. Set by the controller when the
    /// first pod is created. `pastActiveDeadline` is `false` until this
    /// is populated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<DateTime<Utc>>,
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

/// Returns true once `now - status.start_time >= active_deadline_seconds`.
/// Mirrors `pkg/controller/job/job_controller.go::pastActiveDeadline`:
///
/// ```go
/// func (jm *Controller) pastActiveDeadline(job *batch.Job) bool {
///     if job.Spec.ActiveDeadlineSeconds == nil ||
///        job.Status.StartTime == nil ||
///        jobSuspended(job) {
///         return false
///     }
///     duration := jm.clock.Since(job.Status.StartTime.Time)
///     allowedDuration := time.Duration(*job.Spec.ActiveDeadlineSeconds) * time.Second
///     return duration >= allowedDuration
/// }
/// ```
///
/// `now` is passed in (rather than calling `Utc::now`) so tests can
/// pin the clock — same pattern as the upstream `clock.Clock` interface.
pub fn past_active_deadline(spec: &JobSpec, status: &JobStatus, now: DateTime<Utc>) -> bool {
    if spec.suspended {
        return false;
    }
    let Some(deadline) = spec.active_deadline_seconds else {
        return false;
    };
    let Some(t0) = status.start_time else {
        return false;
    };
    (now - t0) >= Duration::seconds(deadline)
}

/// Mirrors `manageJob` in `pkg/controller/job/job_controller.go`. Returns the
/// number of pods to create (clamped by parallelism and remaining completions).
///
/// Uses [`Utc::now`] for the activeDeadlineSeconds clock; tests that need a
/// deterministic clock should call [`reconcile_with_clock`].
pub fn reconcile(
    spec: &JobSpec,
    status: &JobStatus,
    tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    reconcile_with_clock(spec, status, tenant, Utc::now())
}

/// Same as [`reconcile`] but with an injectable clock.
///
/// Upstream behaviour for `pastActiveDeadline`:
///   * The controller marks the Job failed with reason
///     `JobReasonDeadlineExceeded` and deletes every active pod.
///   * Subsequent passes find `status.active == 0` and emit no further
///     work, while the failure condition remains sticky on the status.
///
/// Mirrors `Controller.syncJob`'s ordering: deadline check sits *after*
/// the `suspended` short-circuit and *before* the parallelism / completions
/// calculation, so a deadline-expired Job never spawns new pods.
pub fn reconcile_with_clock(
    spec: &JobSpec,
    status: &JobStatus,
    _tenant: &TenantId,
    now: DateTime<Utc>,
) -> Result<Reconcile, ControllerError> {
    if spec.suspended {
        // Suspended jobs delete any active pods.
        return Ok(if status.active > 0 {
            Reconcile::Delete(status.active)
        } else {
            Reconcile::NoOp
        });
    }
    if past_active_deadline(spec, status, now) {
        // Deadline exceeded — drain any active pods; once drained, NoOp.
        return Ok(if status.active > 0 {
            Reconcile::Delete(status.active)
        } else {
            Reconcile::NoOp
        });
    }
    if is_complete(spec, status) || past_backoff(spec, status) {
        return Ok(Reconcile::NoOp);
    }
    let remaining = spec
        .completions
        .saturating_sub(status.succeeded + status.active);
    let want = remaining.min(spec.parallelism.saturating_sub(status.active));
    if want == 0 {
        Ok(Reconcile::NoOp)
    } else {
        Ok(Reconcile::Create(want))
    }
}

/// Per-index status for an Indexed-completion Job. Mirrors the
/// `JobStatus.UncountedTerminatedPods` + `CompletedIndexes` model from
/// `pkg/controller/job/indexed_job_utils.go`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexState {
    Pending,
    Active,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedJobStatus {
    /// 0-based index → state. Length equals `spec.completions`.
    pub indexes: Vec<IndexState>,
}

impl IndexedJobStatus {
    pub fn new_pending(completions: u32) -> Self {
        Self {
            indexes: (0..completions).map(|_| IndexState::Pending).collect(),
        }
    }

    pub fn count(&self, want: &IndexState) -> u32 {
        self.indexes
            .iter()
            .filter(|s| std::mem::discriminant(*s) == std::mem::discriminant(want))
            .count() as u32
    }
}

/// Plan one indexed-job step. Mirrors
/// `pkg/controller/job/indexed_job_utils.go::firstPendingIndexes`:
///   * never exceed `parallelism` concurrent active indexes,
///   * pick the lowest pending indexes first,
///   * skip already-Succeeded/Failed slots.
pub fn index_status(
    spec: &JobSpec,
    status: &IndexedJobStatus,
) -> Result<Vec<u32>, ControllerError> {
    if status.indexes.len() != spec.completions as usize {
        return Err(ControllerError::InvalidSpec {
            kind: "IndexedJob",
            reason: format!(
                "indexed status length {} does not match completions {}",
                status.indexes.len(),
                spec.completions
            ),
        });
    }
    let active = status.count(&IndexState::Active);
    let want = spec.parallelism.saturating_sub(active);
    let mut out = vec![];
    for (i, s) in status.indexes.iter().enumerate() {
        if out.len() as u32 >= want {
            break;
        }
        if matches!(s, IndexState::Pending) {
            out.push(i as u32);
        }
    }
    Ok(out)
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
            active_deadline_seconds: None,
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
        let st = JobStatus {
            active: 0,
            succeeded: 5,
            failed: 0,
            start_time: None,
        };
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
        let st = JobStatus {
            active: 1,
            succeeded: 0,
            failed: 4,
            start_time: None,
        };
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
        let st = JobStatus {
            active: 4,
            succeeded: 0,
            failed: 0,
            start_time: None,
        };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Delete(4));
    }

    // ── Deeper coverage (deeper-001) ─────────────────────────────────────────

    /// Upstream parity: `TestManageJob_RespectsParallelismVsRemaining`
    /// (job_controller_test.go::TestManageJob — when remaining work is
    /// less than parallelism, only the remaining count is launched).
    #[test]
    fn manage_job_caps_at_remaining_when_below_parallelism() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "manageJob",
            "tenant-job-remaining-cap"
        );
        let s = job(
            /*par=*/ 5, /*compl=*/ 8, /*backoff=*/ 6, /*suspend=*/ false,
        );
        let st = JobStatus {
            active: 0,
            succeeded: 7,
            failed: 0,
            start_time: None,
        };
        // remaining = 8 - 7 = 1, parallelism = 5 → launch 1.
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Create(1));
    }

    /// Upstream parity: `TestManageJob_NoOpWhenAtParallelism`
    /// (job_controller_test.go — no new pods launched while active count
    /// equals parallelism, even with remaining completions).
    #[test]
    fn manage_job_is_noop_when_active_equals_parallelism() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "manageJob",
            "tenant-job-at-parallelism"
        );
        let s = job(3, 10, 6, false);
        let st = JobStatus {
            active: 3,
            succeeded: 0,
            failed: 0,
            start_time: None,
        };
        assert_eq!(
            reconcile(&s, &st, &tenant).unwrap(),
            Reconcile::NoOp,
            "no surge while at parallelism cap"
        );
    }

    /// Upstream parity: `TestIndexedJob_FirstPendingIndexes`
    /// (indexed_job_utils_test.go::TestFirstPendingIndexes — picks the
    /// lowest pending indexes up to `parallelism - active`).
    #[test]
    fn indexed_job_picks_lowest_pending_indexes_within_parallelism_budget() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/indexed_job_utils.go",
            "firstPendingIndexes",
            "tenant-job-indexed-pick"
        );
        let _ = tenant;
        let s = job(
            /*par=*/ 3, /*compl=*/ 5, /*backoff=*/ 6, /*suspend=*/ false,
        );
        let st = IndexedJobStatus {
            indexes: vec![
                IndexState::Succeeded, // 0
                IndexState::Pending,   // 1
                IndexState::Active,    // 2 (counts toward parallelism)
                IndexState::Pending,   // 3
                IndexState::Pending,   // 4
            ],
        };
        // active=1, parallelism=3 → want=2 → pick lowest two pending: 1, 3.
        let picks = index_status(&s, &st).unwrap();
        assert_eq!(picks, vec![1, 3]);
    }

    /// Upstream parity: `TestIndexedJob_AllSucceededYieldsNoMoreWork`
    /// (indexed_job_utils_test.go — once every index is Succeeded the
    /// scheduler emits no further work).
    #[test]
    fn indexed_job_emits_nothing_when_all_indexes_succeeded() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/indexed_job_utils.go",
            "firstPendingIndexes",
            "tenant-job-indexed-done"
        );
        let _ = tenant;
        let s = job(3, 4, 6, false);
        let st = IndexedJobStatus {
            indexes: vec![IndexState::Succeeded; 4],
        };
        let picks = index_status(&s, &st).unwrap();
        assert!(picks.is_empty());
    }

    /// Upstream parity: `TestIndexedJob_RejectsInconsistentStatusLength`
    /// (indexed_job_utils.go — status indexes length must match
    /// `spec.completions`).
    #[test]
    fn indexed_job_rejects_status_length_mismatch() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/indexed_job_utils.go",
            "validateIndexedJobStatus",
            "tenant-job-indexed-inconsistent"
        );
        let _ = tenant;
        let s = job(3, 5, 6, false);
        let st = IndexedJobStatus {
            indexes: vec![IndexState::Pending; 3],
        }; // 3 != 5
        let err = index_status(&s, &st).unwrap_err();
        assert!(matches!(
            err,
            ControllerError::InvalidSpec {
                kind: "IndexedJob",
                ..
            }
        ));
    }

    /// Upstream parity: `TestPastBackoffLimit_BoundaryAtEqual`
    /// (job_controller_test.go::TestPastBackoffLimitOnFailure — exactly
    /// `failed == backoff_limit` is NOT past the limit yet).
    #[test]
    fn past_backoff_is_strictly_greater_than_limit() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "pastBackoffLimitOnFailure",
            "tenant-job-backoff-equal"
        );
        let _ = tenant;
        let s = job(2, 10, /*backoff=*/ 3, false);
        let on_edge = JobStatus {
            active: 0,
            succeeded: 0,
            failed: 3,
            start_time: None,
        };
        assert!(
            !past_backoff(&s, &on_edge),
            "failed == backoff_limit is at the boundary, not past"
        );
        let over = JobStatus {
            active: 0,
            succeeded: 0,
            failed: 4,
            start_time: None,
        };
        assert!(past_backoff(&s, &over));
    }
}
