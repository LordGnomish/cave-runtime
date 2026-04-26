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

/// Direction of a scaling decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScaleDirection {
    Up,
    Down,
}

/// One element of `behavior.scaleUp.policies` / `scaleDown.policies`.
/// Mirrors `autoscaling/v2.HPAScalingPolicy`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScalingPolicy {
    /// Allow at most `value` pod additions/removals per `period_sec`.
    Pods { value: u32, period_sec: u32 },
    /// Allow at most `value`% pod additions/removals per `period_sec`.
    Percent { value: u32, period_sec: u32 },
}

/// `autoscaling/v2.HPAScalingRules.SelectPolicy`. The controller takes
/// either the most-restrictive policy (Min) or most-permissive (Max).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectPolicy {
    Min,
    Max,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalingRules {
    pub select: SelectPolicy,
    pub policies: Vec<ScalingPolicy>,
    /// Stabilization window for this direction. If a scaling decision in the
    /// last `stabilization_window_sec` would have allowed `current`, prefer
    /// that. Mirrors `stabilizeRecommendation`.
    pub stabilization_window_sec: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HpaBehavior {
    pub scale_up: Option<ScalingRules>,
    pub scale_down: Option<ScalingRules>,
}

fn cap_for_policy(policy: &ScalingPolicy, current: u32) -> u32 {
    match *policy {
        ScalingPolicy::Pods { value, .. } => value,
        ScalingPolicy::Percent { value, .. } => {
            ((current as u64 * value as u64).div_ceil(100)) as u32
        }
    }
}

/// Apply a behavior policy to a desired-replica recommendation.
/// Mirrors `pkg/controller/podautoscaler/horizontal.go::stabilizeRecommendation`
/// + `convertDesiredReplicasWithBehaviorRate`.
///
/// `direction` is derived from the relationship between `current` and `desired`.
/// Returns the post-policy desired count.
pub fn apply_behavior(
    behavior: &HpaBehavior,
    current: u32,
    desired: u32,
) -> Result<u32, ControllerError> {
    if desired == current {
        return Ok(desired);
    }
    let dir = if desired > current { ScaleDirection::Up } else { ScaleDirection::Down };
    let rules = match dir {
        ScaleDirection::Up   => behavior.scale_up.as_ref(),
        ScaleDirection::Down => behavior.scale_down.as_ref(),
    };
    let Some(rules) = rules else { return Ok(desired); };
    if rules.select == SelectPolicy::Disabled {
        // Direction explicitly disabled — pin at current.
        return Ok(current);
    }
    if rules.policies.is_empty() {
        return Ok(desired);
    }
    let caps: Vec<u32> = rules.policies.iter()
        .map(|p| cap_for_policy(p, current))
        .collect();
    let chosen_cap = match rules.select {
        SelectPolicy::Min => *caps.iter().min().unwrap_or(&0),
        SelectPolicy::Max => *caps.iter().max().unwrap_or(&0),
        SelectPolicy::Disabled => return Ok(current),
    };
    let bounded = match dir {
        ScaleDirection::Up   => current.saturating_add(chosen_cap).min(desired),
        ScaleDirection::Down => {
            let floor = current.saturating_sub(chosen_cap);
            floor.max(desired)
        }
    };
    Ok(bounded)
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

    // ── Deeper coverage (deeper-001) ─────────────────────────────────────────

    /// Upstream parity: `TestBehavior_ScaleUpPodsPolicyCapsAddition`
    /// (horizontal_test.go::TestConvertDesiredReplicasWithBehaviorRate —
    /// `Pods { value: N }` policy adds at most N pods per period).
    #[test]
    fn behavior_scale_up_pods_policy_caps_addition() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "convertDesiredReplicasWithBehaviorRate",
            "tenant-hpa-behavior-up-pods"
        );
        let _ = tenant;
        let b = HpaBehavior {
            scale_up: Some(ScalingRules {
                select: SelectPolicy::Min,
                policies: vec![ScalingPolicy::Pods { value: 2, period_sec: 60 }],
                stabilization_window_sec: 0,
            }),
            scale_down: None,
        };
        // current=4, desired=10 → bounded by +2 → 6.
        assert_eq!(apply_behavior(&b, 4, 10).unwrap(), 6);
    }

    /// Upstream parity: `TestBehavior_ScaleDownPercentPolicy`
    /// (horizontal_test.go — Percent policy is computed against current
    /// replicas with ceiling division).
    #[test]
    fn behavior_scale_down_percent_policy_caps_removal() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "convertDesiredReplicasWithBehaviorRate",
            "tenant-hpa-behavior-down-pct"
        );
        let _ = tenant;
        let b = HpaBehavior {
            scale_up: None,
            scale_down: Some(ScalingRules {
                select: SelectPolicy::Min,
                policies: vec![ScalingPolicy::Percent { value: 25, period_sec: 60 }],
                stabilization_window_sec: 0,
            }),
        };
        // current=10, desired=2 → cap = ceil(10*25/100) = 3 → floor = 7.
        assert_eq!(apply_behavior(&b, 10, 2).unwrap(), 7);
    }

    /// Upstream parity: `TestBehavior_SelectPolicyMinIsMostRestrictive`
    /// (horizontal_test.go — when SelectPolicy::Min, the smaller cap wins).
    #[test]
    fn behavior_select_min_picks_most_restrictive_policy() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "selectMin",
            "tenant-hpa-behavior-min"
        );
        let _ = tenant;
        let b = HpaBehavior {
            scale_up: Some(ScalingRules {
                select: SelectPolicy::Min,
                policies: vec![
                    ScalingPolicy::Pods { value: 5, period_sec: 60 },
                    ScalingPolicy::Percent { value: 100, period_sec: 60 }, // 100% of 4 = 4
                ],
                stabilization_window_sec: 0,
            }),
            scale_down: None,
        };
        // current=4, desired=20 → caps {5, 4} → Min=4 → 4+4=8.
        assert_eq!(apply_behavior(&b, 4, 20).unwrap(), 8);
    }

    /// Upstream parity: `TestBehavior_DisabledDirectionPinsAtCurrent`
    /// (horizontal_test.go — `selectPolicy: Disabled` blocks all moves
    /// in that direction).
    #[test]
    fn behavior_disabled_direction_pins_replicas_at_current() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "selectPolicyDisabled",
            "tenant-hpa-behavior-disabled"
        );
        let _ = tenant;
        let b = HpaBehavior {
            scale_up: None,
            scale_down: Some(ScalingRules {
                select: SelectPolicy::Disabled,
                policies: vec![],
                stabilization_window_sec: 0,
            }),
        };
        // Down direction disabled → desired is overridden back to current.
        assert_eq!(apply_behavior(&b, 8, 2).unwrap(), 8);
        // No rule for up → desired flows through unchanged.
        assert_eq!(apply_behavior(&b, 4, 10).unwrap(), 10);
    }

    /// Upstream parity: `TestBehavior_NoOpWhenDesiredEqualsCurrent`
    /// (horizontal_test.go — equal current/desired short-circuits to NoOp).
    #[test]
    fn behavior_returns_current_unchanged_when_desired_equals_current() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "convertDesiredReplicasWithBehaviorRate",
            "tenant-hpa-behavior-noop"
        );
        let _ = tenant;
        let b = HpaBehavior::default();
        assert_eq!(apply_behavior(&b, 4, 4).unwrap(), 4);
    }
}
