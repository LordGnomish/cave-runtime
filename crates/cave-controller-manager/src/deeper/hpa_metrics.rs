//! HPA metric ingest — `pkg/controller/podautoscaler/replica_calculator.go`.
//!
//! Mirrors the sub-pipeline that goes from raw `metrics-server` ingest to the
//! per-pod usage value used by `GetResourceReplicas` / `GetMetricReplicas`.
//!
//! Key behaviors:
//!
//! * **Missing pods**: pods with no metric reading. Upstream excludes them
//!   from the average, then re-adds them after the desired-replica computation
//!   with one of two assumptions controlled by direction:
//!     * scale-up: missing pods assumed to be at 0% utilization (worst-case-
//!       for-scale-up — reduces aggressiveness).
//!     * scale-down: missing pods assumed to be at 100% utilization (worst-
//!       case-for-scale-down — keeps replicas).
//! * **Unready pods**: pods present but `Ready != True`. Excluded from the
//!   average when scaling up (to avoid blowing up replicas based on
//!   not-yet-warmed-up traffic).

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricKind {
    /// Per-pod resource (cpu/memory) — `autoscaling/v2.ResourceMetricSource`.
    Resource,
    /// Per-pod custom metric — `PodsMetricSource`.
    Pods,
    /// Single object metric (Service, Ingress) — `ObjectMetricSource`.
    Object,
    /// External metric source (cloud provider queue depth, etc.).
    External,
    /// Per-container resource — `ContainerResourceMetricSource`.
    ContainerResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodMetric {
    pub name: String,
    /// `Ready` condition on the pod.
    pub ready: bool,
    /// `None` when `metrics-server` returned no reading for this pod.
    pub value: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScaleDirection {
    Up,
    Down,
}

/// Group pods by their measurement state. Pods with `value: None` go to
/// `missing`; ready pods with values go to `ready`; pods present but not
/// `Ready` go to `unready`.
pub fn partition(pods: &[PodMetric]) -> (Vec<&PodMetric>, Vec<&PodMetric>, Vec<&PodMetric>) {
    let mut ready = vec![];
    let mut unready = vec![];
    let mut missing = vec![];
    for p in pods {
        match (p.value, p.ready) {
            (None, _) => missing.push(p),
            (Some(_), false) => unready.push(p),
            (Some(_), true) => ready.push(p),
        }
    }
    (ready, unready, missing)
}

/// Mean usage across the ready pods. Returns `None` when there are no ready
/// pods (caller decides what to do — usually error or NoOp).
pub fn ready_average(pods: &[PodMetric]) -> Option<f64> {
    let (ready, _, _) = partition(pods);
    if ready.is_empty() {
        return None;
    }
    let sum: u64 = ready.iter().map(|p| p.value.unwrap_or(0)).sum();
    Some(sum as f64 / ready.len() as f64)
}

/// Computes the desired replica count using the upstream `GetResourceReplicas`
/// rules with missing/unready awareness.
///
/// Steps mirror upstream:
/// 1. Compute usageRatio over the ready pods.
/// 2. Naive desired = ceil(readyCount * usageRatio).
/// 3. If scaling up: re-add missing pods at 0% (count, but at zero usage).
///    Recompute ratio over (ready + missing@0%) and clip the desired.
/// 4. If scaling down: re-add missing pods at target (forces them to count,
///    keeping replicas).
pub fn replicas_for_metric(
    pods: &[PodMetric],
    target_per_pod: u64,
    current_replicas: u32,
) -> Result<u32, ControllerError> {
    if target_per_pod == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "target_per_pod must be > 0".into(),
        });
    }
    let (ready, unready, missing) = partition(pods);
    if ready.is_empty() && unready.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "no ready or unready pods to compute metric".into(),
        });
    }
    if ready.is_empty() {
        // All pods unready/missing → conservative: keep current.
        return Ok(current_replicas);
    }
    let ready_sum: u64 = ready.iter().map(|p| p.value.unwrap_or(0)).sum();
    // Naive ratio over the ready set:
    let ratio = ready_sum as f64 / (target_per_pod as f64 * ready.len() as f64);
    let direction = if ratio > 1.0 { ScaleDirection::Up } else { ScaleDirection::Down };

    // Add missing/unready into the per-pod denominator according to direction.
    let total = match direction {
        // Scale-up: missing@0%, unready@0% — INFLATES denominator → shrinks ratio.
        ScaleDirection::Up => {
            let zero_count = (missing.len() + unready.len()) as f64;
            let new_ratio_num = ready_sum as f64;
            let new_ratio_den = target_per_pod as f64 * (ready.len() as f64 + zero_count);
            new_ratio_num / new_ratio_den
        }
        // Scale-down: missing@100% of target — INFLATES numerator by missing*target,
        // resists scale-down.
        ScaleDirection::Down => {
            let missing_count = missing.len() as f64;
            let new_ratio_num = ready_sum as f64 + (target_per_pod as f64 * missing_count);
            let new_ratio_den = target_per_pod as f64 * (ready.len() as f64 + missing_count);
            new_ratio_num / new_ratio_den
        }
    };
    // ceil(current * total)
    let proposed = (current_replicas as f64 * total).ceil() as u32;
    Ok(proposed)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/podautoscaler/replica_calculator.go",
    "GetResourceReplicas",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn pm(name: &str, ready: bool, value: Option<u64>) -> PodMetric {
        PodMetric { name: name.into(), ready, value }
    }

    #[test]
    fn partition_separates_ready_unready_and_missing() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "groupPods",
            "tenant-hpa-metric-partition"
        );
        let pods = vec![
            pm("a", true, Some(50)),
            pm("b", false, Some(80)),
            pm("c", true, None),
            pm("d", true, Some(30)),
        ];
        let (r, u, m) = partition(&pods);
        assert_eq!(r.len(), 2);
        assert_eq!(u.len(), 1);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn ready_average_excludes_missing_and_unready() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "calculatePodRequests",
            "tenant-hpa-metric-avg"
        );
        let pods = vec![
            pm("a", true, Some(40)),
            pm("b", true, Some(60)),
            pm("c", false, Some(200)),  // unready — excluded
            pm("d", true, None),         // missing — excluded
        ];
        assert_eq!(ready_average(&pods), Some(50.0));
    }

    #[test]
    fn ready_average_returns_none_when_no_ready_pods() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "calculatePodRequests",
            "tenant-hpa-metric-avg-none"
        );
        let pods = vec![
            pm("a", false, Some(10)),
            pm("b", true, None),
        ];
        assert_eq!(ready_average(&pods), None);
    }

    #[test]
    fn replicas_for_metric_zero_target_is_invalid() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-metric-zero-target"
        );
        let pods = vec![pm("a", true, Some(50))];
        assert!(replicas_for_metric(&pods, 0, 1).is_err());
    }

    #[test]
    fn replicas_for_metric_no_pods_at_all_is_invalid() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-metric-empty"
        );
        let pods: Vec<PodMetric> = vec![];
        assert!(replicas_for_metric(&pods, 50, 1).is_err());
    }

    #[test]
    fn replicas_for_metric_all_unready_returns_current() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-metric-all-unready"
        );
        let pods = vec![
            pm("a", false, Some(50)),
            pm("b", false, Some(60)),
        ];
        assert_eq!(replicas_for_metric(&pods, 50, 4).unwrap(), 4);
    }

    #[test]
    fn replicas_for_metric_scales_up_on_high_usage() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-metric-scale-up"
        );
        // 4 pods at 100 each, target 50 → ratio=2 → 4*2=8.
        let pods: Vec<_> = (0..4).map(|i| pm(&format!("p{i}"), true, Some(100))).collect();
        assert_eq!(replicas_for_metric(&pods, 50, 4).unwrap(), 8);
    }

    #[test]
    fn replicas_for_metric_scales_down_on_low_usage() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-metric-scale-down"
        );
        // 8 pods at 25 each, target 100 → ratio=0.25 → 8*0.25=2.
        let pods: Vec<_> = (0..8).map(|i| pm(&format!("p{i}"), true, Some(25))).collect();
        assert_eq!(replicas_for_metric(&pods, 100, 8).unwrap(), 2);
    }

    #[test]
    fn replicas_for_metric_missing_pods_dampen_scale_up() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-metric-missing-up"
        );
        // 2 ready @ 100, 2 missing, target 50.
        // Without missing handling → ratio=2 → 4*2=8 replicas.
        // With missing@0% baked into denominator: ratio = 200 / (50*4) = 1.0 → 4 replicas.
        let pods = vec![
            pm("a", true, Some(100)),
            pm("b", true, Some(100)),
            pm("c", true, None),
            pm("d", true, None),
        ];
        let got = replicas_for_metric(&pods, 50, 4).unwrap();
        assert_eq!(got, 4, "missing pods at 0% should bring scale-up factor to 1.0");
    }

    #[test]
    fn replicas_for_metric_missing_pods_resist_scale_down() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-metric-missing-down"
        );
        // 2 ready @ 25, 2 missing, target 100, current=4.
        // Naive: ratio = 50/(100*2) = 0.25 → would suggest 1.
        // With missing@target: num = 50 + 100*2 = 250, den = 100*4 = 400 → 0.625 → ceil(4*0.625)=3.
        let pods = vec![
            pm("a", true, Some(25)),
            pm("b", true, Some(25)),
            pm("c", true, None),
            pm("d", true, None),
        ];
        let got = replicas_for_metric(&pods, 100, 4).unwrap();
        assert_eq!(got, 3, "missing pods at 100% target should resist scale-down");
    }

    #[test]
    fn replicas_for_metric_unready_pods_dampen_scale_up() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-metric-unready-up"
        );
        // 2 ready @ 100, 2 unready (excluded from sum, added@0% only on scale-up).
        // Scale-up: ratio = 200 / (50*(2+2)) = 1.0 → 4 replicas (current=4 → no change).
        let pods = vec![
            pm("a", true, Some(100)),
            pm("b", true, Some(100)),
            pm("c", false, Some(50)),
            pm("d", false, Some(50)),
        ];
        let got = replicas_for_metric(&pods, 50, 4).unwrap();
        assert_eq!(got, 4);
    }

    #[test]
    fn replicas_for_metric_metric_kind_round_trip_serde() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "MetricSpec",
            "tenant-hpa-metric-kind-serde"
        );
        for k in [
            MetricKind::Resource,
            MetricKind::Pods,
            MetricKind::Object,
            MetricKind::External,
            MetricKind::ContainerResource,
        ] {
            let s = serde_json::to_string(&k).unwrap();
            let back: MetricKind = serde_json::from_str(&s).unwrap();
            assert_eq!(k, back);
        }
    }
}
