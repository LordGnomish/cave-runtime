// SPDX-License-Identifier: AGPL-3.0-or-later
//! TTLAfterFinished controller — `pkg/controller/ttlafterfinished`.
//!
//! Reads `Job.spec.ttlSecondsAfterFinished` and deletes the Job (with
//! Background propagation) when `finished_at + ttl` has elapsed. Mirrors
//! `processJob` and `timeLeft`.

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishedReason {
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinishedJob {
    pub name: String,
    pub namespace: String,
    /// `None` means the Job is still running — TTL doesn't apply yet.
    pub finished_at_sec: Option<u64>,
    pub finished_reason: Option<FinishedReason>,
    /// `spec.ttlSecondsAfterFinished`. `None` means TTL is opt-out.
    pub ttl_sec: Option<u32>,
}

/// Outcome of one TTL evaluation pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TtlAction {
    /// Delete this Job now (possibly with Background propagation).
    DeleteNow,
    /// Re-queue after `secs` seconds — Job not yet expired.
    RequeueAfter(u64),
    /// Skip — Job is still running, or has no TTL.
    Skip,
}

/// Compute time remaining until expiry. Mirrors upstream `timeLeft`:
///
/// `expire_at = finished_at + ttl`
///
/// Returns `Ok(None)` if the Job is not eligible (still running or no TTL).
pub fn time_left(job: &FinishedJob, now_sec: u64) -> Result<Option<i64>, ControllerError> {
    let Some(finished_at) = job.finished_at_sec else {
        return Ok(None);
    };
    let Some(ttl) = job.ttl_sec else {
        return Ok(None);
    };
    let expire = finished_at as i64 + ttl as i64;
    Ok(Some(expire - now_sec as i64))
}

/// Decide what to do with `job` at `now_sec`. Mirrors `processJob`.
pub fn evaluate(job: &FinishedJob, now_sec: u64) -> Result<TtlAction, ControllerError> {
    let Some(left) = time_left(job, now_sec)? else {
        return Ok(TtlAction::Skip);
    };
    if left <= 0 {
        Ok(TtlAction::DeleteNow)
    } else {
        Ok(TtlAction::RequeueAfter(left as u64))
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
    "Controller",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn job(finished_at: Option<u64>, ttl: Option<u32>) -> FinishedJob {
        FinishedJob {
            name: "j".into(),
            namespace: "default".into(),
            finished_at_sec: finished_at,
            finished_reason: finished_at.map(|_| FinishedReason::Complete),
            ttl_sec: ttl,
        }
    }

    #[test]
    fn running_job_is_skipped() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "processJob",
            "tenant-ttl-skip-running"
        );
        let j = job(None, Some(60));
        assert_eq!(evaluate(&j, 100).unwrap(), TtlAction::Skip);
    }

    #[test]
    fn finished_job_without_ttl_is_skipped() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "processJob",
            "tenant-ttl-skip-no-ttl"
        );
        let j = job(Some(50), None);
        assert_eq!(evaluate(&j, 100).unwrap(), TtlAction::Skip);
    }

    #[test]
    fn unexpired_job_requeues_with_remaining_seconds() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "timeLeft",
            "tenant-ttl-requeue"
        );
        // finished=100, ttl=60 → expires at 160. Now=130 → 30s left.
        let j = job(Some(100), Some(60));
        assert_eq!(evaluate(&j, 130).unwrap(), TtlAction::RequeueAfter(30));
    }

    #[test]
    fn expired_job_deletes_now() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "processJob",
            "tenant-ttl-delete-expired"
        );
        let j = job(Some(100), Some(60));
        // Now=200 — expired by 40s.
        assert_eq!(evaluate(&j, 200).unwrap(), TtlAction::DeleteNow);
    }

    #[test]
    fn job_at_exact_expiry_deletes_now() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "processJob",
            "tenant-ttl-delete-edge"
        );
        let j = job(Some(100), Some(60));
        assert_eq!(evaluate(&j, 160).unwrap(), TtlAction::DeleteNow);
    }

    #[test]
    fn ttl_zero_deletes_immediately_after_finish() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "processJob",
            "tenant-ttl-zero-ttl"
        );
        let j = job(Some(100), Some(0));
        assert_eq!(evaluate(&j, 100).unwrap(), TtlAction::DeleteNow);
    }

    #[test]
    fn time_left_returns_none_for_unfinished_job() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "timeLeft",
            "tenant-ttl-time-left-running"
        );
        let j = job(None, Some(60));
        assert_eq!(time_left(&j, 100).unwrap(), None);
    }

    #[test]
    fn time_left_negative_means_expired() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "timeLeft",
            "tenant-ttl-time-left-negative"
        );
        let j = job(Some(100), Some(10));
        assert_eq!(time_left(&j, 200).unwrap(), Some(-90));
    }

    #[test]
    fn ttl_action_serializes_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "TtlAction",
            "tenant-ttl-action-serde"
        );
        for a in [
            TtlAction::DeleteNow,
            TtlAction::RequeueAfter(5),
            TtlAction::Skip,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: TtlAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }

    #[test]
    fn finished_reason_round_trips_serde() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/batch/v1/types.go",
            "JobConditionType",
            "tenant-ttl-reason-serde"
        );
        for r in [FinishedReason::Complete, FinishedReason::Failed] {
            let s = serde_json::to_string(&r).unwrap();
            let back: FinishedReason = serde_json::from_str(&s).unwrap();
            assert_eq!(r, back);
        }
    }
}
