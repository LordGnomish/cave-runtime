// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HPA scaling-behavior advanced helpers.
//!
//! Extends [`crate::hpa::HpaBehavior`] with the upstream details that the
//! deeper-001 layer didn't yet cover:
//!
//! * `longestPolicyPeriod` — the periodicity of recommendations is the longest
//!   `period_sec` across all policies; relevant for stabilization windows.
//! * Default select policy — `nil` SelectPolicy in upstream defaults to `Max`
//!   (most-permissive) for both directions.
//! * Direction defaults: scale-up has no stabilization (`0s`), scale-down has
//!   the kube-controller-manager flag default of 300s.
//! * Spec validation — upstream `validateHorizontalPodAutoscalerSpec` rejects
//!   `min > max`, `min == 0` without `min_replicas` permission, etc.

use crate::hpa::{HpaBehavior, ScalingPolicy, ScalingRules, SelectPolicy};
use crate::types::{Cite, ControllerError};

/// Upstream `--horizontal-pod-autoscaler-downscale-stabilization` default.
pub const DEFAULT_DOWNSCALE_STABILISATION_SEC: u32 = 300;
/// Upstream default for scale-up stabilization.
pub const DEFAULT_UPSCALE_STABILISATION_SEC: u32 = 0;

/// Returns the longest `period_sec` across all policies in the rule.
/// Mirrors `longestPolicyPeriod` in `pkg/controller/podautoscaler/horizontal.go`.
pub fn longest_policy_period(rules: &ScalingRules) -> u32 {
    rules
        .policies
        .iter()
        .map(|p| match *p {
            ScalingPolicy::Pods { period_sec, .. } => period_sec,
            ScalingPolicy::Percent { period_sec, .. } => period_sec,
        })
        .max()
        .unwrap_or(0)
}

/// Returns the SelectPolicy actually used. `Disabled` keeps its semantics; in
/// upstream a missing/nil SelectPolicy defaults to `Max`.
pub fn effective_select(rules: &ScalingRules) -> SelectPolicy {
    // Caller is expected to set SelectPolicy explicitly; this helper exists
    // for the case where future API marshalling produces a nil-equivalent.
    // In our typed model the only "nil" cases are when `policies.is_empty()`,
    // which yields a no-op anyway. Pass through.
    rules.select
}

/// Returns the default behavior — used when the user supplied no `behavior`
/// block. Upstream: `pkg/controller/podautoscaler/horizontal.go::generateScalingRules`.
pub fn default_behavior() -> HpaBehavior {
    HpaBehavior {
        scale_up: Some(ScalingRules {
            select: SelectPolicy::Max,
            policies: vec![
                // Scale up by 100% every 15s.
                ScalingPolicy::Percent {
                    value: 100,
                    period_sec: 15,
                },
                // Or by 4 pods every 15s.
                ScalingPolicy::Pods {
                    value: 4,
                    period_sec: 15,
                },
            ],
            stabilization_window_sec: DEFAULT_UPSCALE_STABILISATION_SEC,
        }),
        scale_down: Some(ScalingRules {
            select: SelectPolicy::Max,
            policies: vec![
                // Scale down by 100% every 15s (default permissive).
                ScalingPolicy::Percent {
                    value: 100,
                    period_sec: 15,
                },
            ],
            stabilization_window_sec: DEFAULT_DOWNSCALE_STABILISATION_SEC,
        }),
    }
}

/// Validate spec invariants beyond the immediate `desired_replicas` check.
/// Mirrors `validateHorizontalPodAutoscalerSpec`.
pub fn validate_replica_bounds(min: u32, max: u32) -> Result<(), ControllerError> {
    if max == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "max_replicas must be > 0".into(),
        });
    }
    if min > max {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "min_replicas must be <= max_replicas".into(),
        });
    }
    Ok(())
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/podautoscaler/horizontal.go",
    "generateScalingRules",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn rules_with(policies: Vec<ScalingPolicy>) -> ScalingRules {
        ScalingRules {
            select: SelectPolicy::Max,
            policies,
            stabilization_window_sec: 0,
        }
    }

    #[test]
    fn longest_policy_period_picks_max_across_policies() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "longestPolicyPeriod",
            "tenant-hpa-adv-longest"
        );
        let r = rules_with(vec![
            ScalingPolicy::Pods {
                value: 4,
                period_sec: 30,
            },
            ScalingPolicy::Percent {
                value: 100,
                period_sec: 60,
            },
            ScalingPolicy::Pods {
                value: 2,
                period_sec: 15,
            },
        ]);
        assert_eq!(longest_policy_period(&r), 60);
    }

    #[test]
    fn longest_policy_period_zero_when_empty() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "longestPolicyPeriod",
            "tenant-hpa-adv-longest-empty"
        );
        let r = rules_with(vec![]);
        assert_eq!(longest_policy_period(&r), 0);
    }

    #[test]
    fn longest_policy_period_with_equal_periods() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "longestPolicyPeriod",
            "tenant-hpa-adv-longest-equal"
        );
        let r = rules_with(vec![
            ScalingPolicy::Pods {
                value: 4,
                period_sec: 30,
            },
            ScalingPolicy::Percent {
                value: 100,
                period_sec: 30,
            },
        ]);
        assert_eq!(longest_policy_period(&r), 30);
    }

    #[test]
    fn default_behavior_sets_upstream_windows() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "generateScalingRules",
            "tenant-hpa-adv-default-windows"
        );
        let b = default_behavior();
        assert_eq!(
            b.scale_up.as_ref().unwrap().stabilization_window_sec,
            DEFAULT_UPSCALE_STABILISATION_SEC
        );
        assert_eq!(
            b.scale_down.as_ref().unwrap().stabilization_window_sec,
            DEFAULT_DOWNSCALE_STABILISATION_SEC
        );
    }

    #[test]
    fn default_behavior_uses_select_max_in_both_directions() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "generateScalingRules",
            "tenant-hpa-adv-default-select-max"
        );
        let b = default_behavior();
        assert_eq!(b.scale_up.unwrap().select, SelectPolicy::Max);
        assert_eq!(b.scale_down.unwrap().select, SelectPolicy::Max);
    }

    #[test]
    fn default_behavior_scale_up_has_two_policies() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "generateScalingRules",
            "tenant-hpa-adv-default-up-policies"
        );
        let b = default_behavior();
        assert_eq!(b.scale_up.unwrap().policies.len(), 2);
    }

    #[test]
    fn validate_rejects_max_zero() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/autoscaling/validation/validation.go",
            "validateHorizontalPodAutoscalerSpec",
            "tenant-hpa-adv-validate-max-zero"
        );
        assert!(validate_replica_bounds(0, 0).is_err());
    }

    #[test]
    fn validate_rejects_min_greater_than_max() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/autoscaling/validation/validation.go",
            "validateHorizontalPodAutoscalerSpec",
            "tenant-hpa-adv-validate-min-gt-max"
        );
        assert!(validate_replica_bounds(10, 5).is_err());
    }

    #[test]
    fn validate_admits_min_equals_max() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/autoscaling/validation/validation.go",
            "validateHorizontalPodAutoscalerSpec",
            "tenant-hpa-adv-validate-equal"
        );
        assert!(validate_replica_bounds(5, 5).is_ok());
    }

    #[test]
    fn validate_admits_min_zero_with_max_nonzero() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/autoscaling/validation/validation.go",
            "validateHorizontalPodAutoscalerSpec",
            "tenant-hpa-adv-validate-min-zero"
        );
        // min=0 valid since v1.16 (HPAScaleToZero feature gate).
        assert!(validate_replica_bounds(0, 10).is_ok());
    }

    #[test]
    fn effective_select_passes_through_disabled() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/horizontal.go",
            "selectPolicyDisabled",
            "tenant-hpa-adv-effective-disabled"
        );
        let r = ScalingRules {
            select: SelectPolicy::Disabled,
            policies: vec![ScalingPolicy::Pods {
                value: 1,
                period_sec: 60,
            }],
            stabilization_window_sec: 0,
        };
        assert_eq!(effective_select(&r), SelectPolicy::Disabled);
    }
}
