// SPDX-License-Identifier: AGPL-3.0-or-later
//! HPA multi-metric reconciliation — `pkg/controller/podautoscaler/horizontal.go::computeReplicasForMetrics`.
//!
//! When an HPA carries multiple `metrics[]` sources, the controller computes
//! the desired replica count for each independently and takes the **max** of
//! the per-metric recommendations. If any metric source fails to produce a
//! reading, that source is skipped (and contributes a `ScalingActive=False`
//! condition message), but other sources still drive the recommendation.
//!
//! At least one source must produce a recommendation; otherwise the desired
//! count falls back to `current_replicas`.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricOutcome {
    /// Metric source reported a per-metric desired replica count.
    Replicas(u32),
    /// Metric source returned an error or was un-evaluatable.
    Failed(String),
}

/// Combine per-metric outcomes into one desired replica count.
/// Mirrors the loop in `computeReplicasForMetrics`.
///
/// Returns `(desired, num_failed, succeeded_at_least_once)`.
pub fn combine(
    outcomes: &[MetricOutcome],
    current_replicas: u32,
) -> (u32, u32, bool) {
    let mut max_rec: Option<u32> = None;
    let mut failed = 0u32;
    for o in outcomes {
        match o {
            MetricOutcome::Replicas(r) => {
                max_rec = Some(match max_rec {
                    Some(prev) => prev.max(*r),
                    None => *r,
                });
            }
            MetricOutcome::Failed(_) => failed += 1,
        }
    }
    match max_rec {
        Some(r) => (r, failed, true),
        None => (current_replicas, failed, false),
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/podautoscaler/horizontal.go",
    "computeReplicasForMetrics",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    #[test]
    fn single_metric_drives_recommendation() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "computeReplicasForMetrics",
            "tenant-hpa-multi-single"
        );
        let (rec, failed, ok) = combine(&[MetricOutcome::Replicas(8)], 4);
        assert_eq!(rec, 8);
        assert_eq!(failed, 0);
        assert!(ok);
    }

    #[test]
    fn max_of_multiple_replicas_wins() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "computeReplicasForMetrics",
            "tenant-hpa-multi-max"
        );
        let (rec, _, _) = combine(
            &[
                MetricOutcome::Replicas(5),
                MetricOutcome::Replicas(8),
                MetricOutcome::Replicas(3),
            ],
            4,
        );
        assert_eq!(rec, 8);
    }

    #[test]
    fn all_failures_falls_back_to_current() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "computeReplicasForMetrics",
            "tenant-hpa-multi-all-fail"
        );
        let (rec, failed, ok) = combine(
            &[
                MetricOutcome::Failed("metrics-server timeout".into()),
                MetricOutcome::Failed("custom api 503".into()),
            ],
            6,
        );
        assert_eq!(rec, 6);
        assert_eq!(failed, 2);
        assert!(!ok);
    }

    #[test]
    fn mixed_one_success_drives_recommendation() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "computeReplicasForMetrics",
            "tenant-hpa-multi-mixed"
        );
        let (rec, failed, ok) = combine(
            &[
                MetricOutcome::Failed("noisy".into()),
                MetricOutcome::Replicas(7),
            ],
            4,
        );
        assert_eq!(rec, 7);
        assert_eq!(failed, 1);
        assert!(ok);
    }

    #[test]
    fn empty_metrics_falls_back_to_current_with_zero_failures() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "computeReplicasForMetrics",
            "tenant-hpa-multi-empty"
        );
        let (rec, failed, ok) = combine(&[], 5);
        assert_eq!(rec, 5);
        assert_eq!(failed, 0);
        assert!(!ok);
    }

    #[test]
    fn zero_recommendation_picked_over_failure() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "computeReplicasForMetrics",
            "tenant-hpa-multi-zero"
        );
        let (rec, _, ok) = combine(
            &[
                MetricOutcome::Replicas(0),
                MetricOutcome::Failed("ignored".into()),
            ],
            4,
        );
        assert_eq!(rec, 0);
        assert!(ok);
    }

    #[test]
    fn outcome_round_trips_serde() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "MetricOutcome",
            "tenant-hpa-multi-serde"
        );
        for o in [
            MetricOutcome::Replicas(3),
            MetricOutcome::Failed("x".into()),
        ] {
            let s = serde_json::to_string(&o).unwrap();
            let back: MetricOutcome = serde_json::from_str(&s).unwrap();
            assert_eq!(o, back);
        }
    }
}
