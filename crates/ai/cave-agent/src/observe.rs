// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-improvement, step 1: observability data analysis. The runtime emits a
//! stream of metric samples (latency, energy, accuracy, ...); this module turns
//! a window of them into summary statistics and flags regressions of a *recent*
//! window against a *baseline* window using a z-score test.
//!
//! OpenJarvis upstream: `jarvis/improve/observe.py`. The live OTLP / Prometheus
//! scrape path is owned by cave-metrics; this is the pure analysis core.

use serde::{Deserialize, Serialize};

/// A flat series of scalar samples for one metric.
pub struct Series {
    values: Vec<f64>,
}

impl Series {
    /// Wrap a vector of samples.
    pub fn from(values: Vec<f64>) -> Self {
        Self { values }
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the series is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Arithmetic mean, or `0.0` when empty.
    pub fn mean(&self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        self.values.iter().sum::<f64>() / self.values.len() as f64
    }

    /// Population standard deviation, or `0.0` when empty.
    pub fn stddev(&self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        let m = self.mean();
        let var = self.values.iter().map(|v| (v - m).powi(2)).sum::<f64>()
            / self.values.len() as f64;
        var.sqrt()
    }

    /// Smallest sample, or `0.0` when empty.
    pub fn min(&self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        self.values.iter().cloned().fold(f64::INFINITY, f64::min)
    }

    /// Largest sample, or `0.0` when empty.
    pub fn max(&self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        self.values.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    }

    /// Nearest-rank percentile (`p` in `0..=100`), or `0.0` when empty.
    pub fn percentile(&self, p: f64) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        let mut v = self.values.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p = p.clamp(0.0, 100.0);
        let rank = (p / 100.0 * v.len() as f64).ceil().max(1.0) as usize;
        v[rank - 1]
    }
}

/// The verdict of a baseline-vs-recent comparison.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionReport {
    /// Mean of the baseline window.
    pub baseline_mean: f64,
    /// Mean of the recent window.
    pub recent_mean: f64,
    /// `(recent_mean - baseline_mean) / baseline_stddev`. Positive ⇒ recent is
    /// larger (assumed worse for latency/energy/cost metrics).
    pub z_score: f64,
    /// Percentage change of the recent mean over the baseline mean.
    pub delta_pct: f64,
    /// True when the recent window is *worse* (larger) and the z-score exceeds
    /// the threshold.
    pub regressed: bool,
}

/// Compare a `recent` window against a `baseline` window. Treats *larger*
/// recent values as regressions (the convention for latency/energy/cost). When
/// the baseline has zero variance, any nonzero increase counts as an infinite
/// z-score and is flagged.
pub fn detect_regression(baseline: &[f64], recent: &[f64], z_threshold: f64) -> RegressionReport {
    let base = Series::from(baseline.to_vec());
    let rec = Series::from(recent.to_vec());
    let baseline_mean = base.mean();
    let recent_mean = rec.mean();
    let std = base.stddev();

    let z_score = if std > 0.0 {
        (recent_mean - baseline_mean) / std
    } else if recent_mean > baseline_mean {
        f64::INFINITY
    } else {
        0.0
    };

    let delta_pct = if baseline_mean != 0.0 {
        (recent_mean - baseline_mean) / baseline_mean * 100.0
    } else {
        0.0
    };

    let regressed = recent_mean > baseline_mean && z_score > z_threshold;

    RegressionReport { baseline_mean, recent_mean, z_score, delta_pct, regressed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_variance_baseline_flags_any_increase() {
        let r = detect_regression(&[100.0, 100.0, 100.0], &[110.0], 3.0);
        assert!(r.z_score.is_infinite());
        assert!(r.regressed);
    }

    #[test]
    fn min_of_single_value() {
        assert_eq!(Series::from(vec![3.0]).min(), 3.0);
    }
}
