//! TTLAfterFinished jitter — `pkg/controller/ttlafterfinished/ttlafterfinished_controller.go::enqueueTTL`.
//!
//! Adding jitter to the TTL requeue prevents thundering-herd deletes when
//! many Jobs finish within the same window. Upstream jitters by up to
//! `JitterFactor = 0.1` (10%) of the remaining TTL.

use crate::gc_lite::ttl_after_finished::{TtlAction, FinishedJob, evaluate};
use crate::types::Cite;

pub const JITTER_FACTOR: f64 = 0.1;

/// Apply jitter to a [`TtlAction::RequeueAfter`] using a deterministic
/// seed (the Job name's hash). Mirrors the loop's `wait.Jitter` helper.
pub fn apply_jitter(action: TtlAction, seed: u64) -> TtlAction {
    match action {
        TtlAction::RequeueAfter(secs) => {
            let max = (secs as f64 * JITTER_FACTOR) as u64;
            if max == 0 {
                return TtlAction::RequeueAfter(secs);
            }
            // Pseudo-random in [0, max]; deterministic on `seed`.
            let extra = seed % (max + 1);
            TtlAction::RequeueAfter(secs + extra)
        }
        other => other,
    }
}

/// Convenience: evaluate + jitter.
pub fn evaluate_with_jitter(
    job: &FinishedJob,
    now_sec: u64,
    seed: u64,
) -> Result<TtlAction, crate::types::ControllerError> {
    let raw = evaluate(job, now_sec)?;
    Ok(apply_jitter(raw, seed))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
    "enqueueTTL",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc_lite::ttl_after_finished::FinishedReason;
    use crate::test_ctx;

    fn job(finished: u64, ttl: u32) -> FinishedJob {
        FinishedJob {
            name: "j".into(),
            namespace: "default".into(),
            finished_at_sec: Some(finished),
            finished_reason: Some(FinishedReason::Complete),
            ttl_sec: Some(ttl),
        }
    }

    #[test]
    fn jitter_keeps_value_within_ten_percent_band() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "enqueueTTL",
            "tenant-ttl-jit-band"
        );
        for seed in 0..100u64 {
            let TtlAction::RequeueAfter(jittered) = apply_jitter(TtlAction::RequeueAfter(1000), seed)
            else {
                panic!("expected RequeueAfter");
            };
            assert!((1000..=1100).contains(&jittered));
        }
    }

    #[test]
    fn jitter_zero_remaining_seconds_unchanged() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "enqueueTTL",
            "tenant-ttl-jit-tiny"
        );
        // 5 seconds * 0.1 = 0 (rounded down) → no jitter possible.
        for seed in 0..10u64 {
            assert_eq!(
                apply_jitter(TtlAction::RequeueAfter(5), seed),
                TtlAction::RequeueAfter(5)
            );
        }
    }

    #[test]
    fn jitter_passthrough_for_non_requeue_actions() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "enqueueTTL",
            "tenant-ttl-jit-passthrough"
        );
        assert_eq!(apply_jitter(TtlAction::DeleteNow, 0), TtlAction::DeleteNow);
        assert_eq!(apply_jitter(TtlAction::Skip, 99), TtlAction::Skip);
    }

    #[test]
    fn jitter_deterministic_for_same_seed() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "enqueueTTL",
            "tenant-ttl-jit-deterministic"
        );
        let a = apply_jitter(TtlAction::RequeueAfter(500), 123);
        let b = apply_jitter(TtlAction::RequeueAfter(500), 123);
        assert_eq!(a, b);
    }

    #[test]
    fn jitter_factor_constant() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "JitterFactor",
            "tenant-ttl-jit-const"
        );
        assert!((JITTER_FACTOR - 0.1).abs() < 1e-9);
    }

    #[test]
    fn evaluate_with_jitter_unexpired_emits_jittered_requeue() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "processJob",
            "tenant-ttl-jit-flow"
        );
        let j = job(100, 600);
        // now=130 → 570 left, +up to 57s jitter.
        let TtlAction::RequeueAfter(secs) = evaluate_with_jitter(&j, 130, 7).unwrap() else {
            panic!("expected RequeueAfter");
        };
        assert!((570..=627).contains(&secs));
    }

    #[test]
    fn evaluate_with_jitter_expired_emits_delete_no_jitter() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/ttlafterfinished/ttlafterfinished_controller.go",
            "processJob",
            "tenant-ttl-jit-expired"
        );
        let j = job(100, 60);
        assert_eq!(
            evaluate_with_jitter(&j, 200, 7).unwrap(),
            TtlAction::DeleteNow
        );
    }
}
