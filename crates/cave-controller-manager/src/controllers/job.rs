//! Job controller — observes Job + its pod outcomes and marks the Job as
//! completed once `succeeded >= completions_required`.
//!
//! Upstream: pkg/controller/job/job_controller.go::syncJob

use crate::models::JobObs;

/// Reconcile a Job. Returns the same observation with the `completed` flag
/// updated based on succeeded count vs. completions_required.
pub fn reconcile_job(mut job: JobObs) -> JobObs {
    if job.succeeded >= job.completions_required {
        job.completed = true;
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn j(succeeded: u32, completions: u32) -> JobObs {
        JobObs {
            uid: Uuid::new_v4(),
            name: "j".into(),
            namespace: "default".into(),
            completions_required: completions,
            succeeded,
            failed: 0,
            completed: false,
        }
    }

    /// Port of `TestJobControllerCompletion` — a Job with succeeded ≥ completions
    /// flips to completed; one short of the threshold stays incomplete.
    #[test]
    fn test_job_controller_completion() {
        let done = reconcile_job(j(3, 3));
        assert!(done.completed);
        let one_more = reconcile_job(j(2, 3));
        assert!(!one_more.completed);
    }

    #[test]
    fn test_job_overshoot_still_completed() {
        let over = reconcile_job(j(5, 3));
        assert!(over.completed);
    }
}
