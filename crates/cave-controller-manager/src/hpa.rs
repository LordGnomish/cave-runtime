//! HorizontalPodAutoscaler controller — scales a target based on metrics.
//!
//! Upstream: [`pkg/controller/podautoscaler`]. The full controller resolves
//! external/object/pods/resource metrics, applies a stabilization window per
//! direction, and respects scale-up/scale-down behavior policies.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HpaSpec {
    pub name: String,
    pub namespace: String,
    pub min_replicas: u32,
    pub max_replicas: u32,
    /// Resource utilization target as a percentage (e.g. 80 = 80%).
    pub target_cpu_utilization_pct: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HpaStatus {
    pub current_replicas: u32,
    pub current_cpu_utilization_pct: u32,
}

/// Compute the desired replica count using the canonical algorithm from
/// `GetResourceReplicas` in `pkg/controller/podautoscaler/replica_calculator.go`:
///
/// ```text
/// desired = ceil(current * (currentMetric / targetMetric))
/// ```
///
/// Then clamps to `[min_replicas, max_replicas]`.
pub fn desired_replicas(spec: &HpaSpec, status: &HpaStatus) -> Result<u32, ControllerError> {
    if spec.target_cpu_utilization_pct == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "target utilization must be > 0".into(),
        });
    }
    if spec.min_replicas > spec.max_replicas {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "min_replicas must be <= max_replicas".into(),
        });
    }
    let cur = status.current_replicas.max(1) as u64;
    let m = status.current_cpu_utilization_pct as u64;
    let t = spec.target_cpu_utilization_pct as u64;
    // ceil(cur * m / t)
    let desired = (cur * m + t - 1) / t;
    let clamped = desired.clamp(spec.min_replicas as u64, spec.max_replicas as u64) as u32;
    Ok(clamped)
}

/// Mirrors `reconcileAutoscaler` in upstream — translates a desired-replica
/// computation into a [`Reconcile`] action.
pub fn reconcile(
    spec: &HpaSpec,
    status: &HpaStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    let desired = desired_replicas(spec, status)?;
    if desired == status.current_replicas {
        return Ok(Reconcile::NoOp);
    }
    Ok(Reconcile::Update(desired))
}

/// Stub: scale-up/scale-down `behavior` policy enforcement. Not implemented.
pub fn apply_behavior(_decision: Reconcile) -> Result<Reconcile, ControllerError> {
    unimplemented!("HPA behavior policy — see pkg/controller/podautoscaler/horizontal.go::stabilizeRecommendation")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/podautoscaler/horizontal.go", "HorizontalController");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn hpa(min: u32, max: u32, target: u32) -> HpaSpec {
        HpaSpec {
            name: "web-hpa".into(),
            namespace: "default".into(),
            min_replicas: min,
            max_replicas: max,
            target_cpu_utilization_pct: target,
        }
    }

    #[test]
    fn scales_up_when_metric_exceeds_target() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-scale-up"
        );
        let s = hpa(1, 10, 50);
        let st = HpaStatus { current_replicas: 4, current_cpu_utilization_pct: 100 };
        // desired = ceil(4 * 100 / 50) = 8
        assert_eq!(desired_replicas(&s, &st).unwrap(), 8);
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Update(8));
    }

    #[test]
    fn scales_down_when_metric_below_target() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-scale-down"
        );
        let s = hpa(1, 10, 80);
        let st = HpaStatus { current_replicas: 8, current_cpu_utilization_pct: 20 };
        // desired = ceil(8 * 20 / 80) = 2
        assert_eq!(desired_replicas(&s, &st).unwrap(), 2);
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Update(2));
    }

    #[test]
    fn clamps_to_max_replicas() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "normalizeDesiredReplicas",
            "tenant-hpa-clamp-max"
        );
        let _ = tenant;
        let s = hpa(1, 5, 50);
        let st = HpaStatus { current_replicas: 5, current_cpu_utilization_pct: 200 };
        assert_eq!(desired_replicas(&s, &st).unwrap(), 5);
    }

    #[test]
    fn rejects_invalid_target_zero() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "validateHorizontalPodAutoscaler",
            "tenant-hpa-bad-target"
        );
        let _ = tenant;
        let s = hpa(1, 5, 0);
        let st = HpaStatus { current_replicas: 1, current_cpu_utilization_pct: 50 };
        assert!(desired_replicas(&s, &st).is_err());
    }
}
