// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HPA tolerance check — `pkg/controller/podautoscaler/replica_calculator.go`.
//!
//! Upstream picks `currentReplicas` (no-op) when the usage ratio is within a
//! tolerance band around 1.0 — separately for scale-up and scale-down. The
//! default tolerance is 10% (`defaultTolerance = 0.1`).

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};

/// Default scale-up tolerance — `pkg/controller/podautoscaler/config/types.go`.
pub const DEFAULT_SCALE_UP_TOLERANCE: f64 = 0.1;

/// Default scale-down tolerance.
pub const DEFAULT_SCALE_DOWN_TOLERANCE: f64 = 0.1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ToleranceBand {
    pub scale_up: f64,
    pub scale_down: f64,
}

impl Default for ToleranceBand {
    fn default() -> Self {
        Self {
            scale_up: DEFAULT_SCALE_UP_TOLERANCE,
            scale_down: DEFAULT_SCALE_DOWN_TOLERANCE,
        }
    }
}

/// Compute `usageRatio = currentUtilization / target`.
/// Mirrors `usageRatio` in `replica_calculator.go::GetResourceReplicas`.
pub fn usage_ratio(current_metric: u64, target_metric: u64) -> Result<f64, ControllerError> {
    if target_metric == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "HorizontalPodAutoscaler",
            reason: "target metric must be > 0".into(),
        });
    }
    Ok(current_metric as f64 / target_metric as f64)
}

/// Returns true if `|ratio - 1.0| <= tolerance`.
/// Direction selects which side of the band — upstream applies asymmetric
/// tolerances (scale-up vs scale-down).
pub fn within_tolerance(ratio: f64, tolerance: f64) -> bool {
    (ratio - 1.0).abs() <= tolerance
}

/// Returns the post-tolerance desired replica count. If the metric is within
/// the appropriate tolerance band, returns `current_replicas` (no-op);
/// otherwise returns `proposed_desired`.
pub fn apply_tolerance(
    band: &ToleranceBand,
    current_replicas: u32,
    current_metric: u64,
    target_metric: u64,
    proposed_desired: u32,
) -> Result<u32, ControllerError> {
    let ratio = usage_ratio(current_metric, target_metric)?;
    let tol = if proposed_desired >= current_replicas {
        band.scale_up
    } else {
        band.scale_down
    };
    if within_tolerance(ratio, tol) {
        Ok(current_replicas)
    } else {
        Ok(proposed_desired)
    }
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

    #[test]
    fn ratio_at_one_is_within_default_tolerance() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "withinTolerance",
            "tenant-hpa-tol-exact"
        );
        assert!(within_tolerance(1.0, DEFAULT_SCALE_UP_TOLERANCE));
    }

    #[test]
    fn ratio_inside_upper_band_is_within_tolerance() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "withinTolerance",
            "tenant-hpa-tol-upper"
        );
        // 5% over target with 10% tolerance → still in band.
        assert!(within_tolerance(1.05, 0.10));
    }

    #[test]
    fn ratio_outside_upper_band_breaks_tolerance() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "withinTolerance",
            "tenant-hpa-tol-upper-out"
        );
        assert!(!within_tolerance(1.15, 0.10));
    }

    #[test]
    fn ratio_inside_lower_band_is_within_tolerance() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "withinTolerance",
            "tenant-hpa-tol-lower"
        );
        assert!(within_tolerance(0.92, 0.10));
    }

    #[test]
    fn ratio_outside_lower_band_breaks_tolerance() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "withinTolerance",
            "tenant-hpa-tol-lower-out"
        );
        assert!(!within_tolerance(0.5, 0.10));
    }

    #[test]
    fn zero_target_is_invalid_spec() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-tol-zero"
        );
        assert!(usage_ratio(50, 0).is_err());
    }

    #[test]
    fn apply_tolerance_returns_current_inside_band() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-tol-apply-noop"
        );
        let band = ToleranceBand::default();
        // current=4, current_metric=55, target=50 → ratio=1.10 → outside (>0.1)?
        // 1.10 - 1.0 = 0.10 → equal to 10% → in band → current.
        // Adjust to clearly inside.
        let got = apply_tolerance(&band, 4, 52, 50, 8).unwrap();
        assert_eq!(got, 4);
    }

    #[test]
    fn apply_tolerance_uses_scale_up_band_when_proposed_higher() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-tol-asymmetric-up"
        );
        // Asymmetric: tight scale-up band, loose scale-down band.
        let band = ToleranceBand { scale_up: 0.05, scale_down: 0.30 };
        // ratio=1.10, scale-up dir, band 0.05 → outside → take proposal.
        let got = apply_tolerance(&band, 4, 110, 100, 6).unwrap();
        assert_eq!(got, 6);
    }

    #[test]
    fn apply_tolerance_uses_scale_down_band_when_proposed_lower() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-tol-asymmetric-down"
        );
        let band = ToleranceBand { scale_up: 0.30, scale_down: 0.05 };
        // ratio=0.90, scale-down dir, band 0.05 → outside → take proposal.
        let got = apply_tolerance(&band, 8, 90, 100, 4).unwrap();
        assert_eq!(got, 4);
    }

    #[test]
    fn apply_tolerance_zero_tolerance_admits_only_exact_match() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podautoscaler/replica_calculator.go",
            "GetResourceReplicas",
            "tenant-hpa-tol-zero-band"
        );
        let band = ToleranceBand { scale_up: 0.0, scale_down: 0.0 };
        assert_eq!(apply_tolerance(&band, 5, 100, 100, 7).unwrap(), 5);
        assert_eq!(apply_tolerance(&band, 5, 101, 100, 7).unwrap(), 7);
    }
}
