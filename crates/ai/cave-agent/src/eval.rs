// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Evaluation tools — OpenJarvis scores every on-device run on four axes:
//! energy, latency, cost, and accuracy. Energy/latency/cost are
//! lower-is-better; accuracy is higher-is-better. [`RunMetrics::score`] folds
//! them into a single 0..1 quality number, and [`Evaluator`] aggregates a fleet
//! of runs (mean, nearest-rank percentile, best-by-score).
//!
//! OpenJarvis upstream: `jarvis/eval/metrics.py`. Live hardware-energy sampling
//! (RAPL / battery counters) is scope-cut; metrics are supplied by the caller.

use serde::{Deserialize, Serialize};

/// The four evaluation axes of a single run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct RunMetrics {
    /// Energy consumed, millijoules (lower better).
    pub energy_mj: f64,
    /// Wall-clock latency, milliseconds (lower better).
    pub latency_ms: f64,
    /// Marginal cost, US dollars (lower better).
    pub cost_usd: f64,
    /// Task accuracy in `0.0..=1.0` (higher better).
    pub accuracy: f64,
}

/// Relative importance of each axis. The four fields should sum to 1.0; the
/// presets guarantee that.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Weights {
    pub energy: f64,
    pub latency: f64,
    pub cost: f64,
    pub accuracy: f64,
}

impl Weights {
    /// Equal 0.25 weight on every axis.
    pub fn balanced() -> Self {
        Self { energy: 0.25, latency: 0.25, cost: 0.25, accuracy: 0.25 }
    }

    /// Accuracy dominates (0.7); the efficiency axes share the rest.
    pub fn accuracy_first() -> Self {
        Self { energy: 0.1, latency: 0.1, cost: 0.1, accuracy: 0.7 }
    }
}

impl RunMetrics {
    /// Composite quality score in `(0, 1]`. Each lower-is-better axis is mapped
    /// to `1 / (1 + x/scale)` (1.0 at zero, decaying toward 0); accuracy passes
    /// through directly. The four terms are combined by `weights`.
    ///
    /// Scales (`energy/1000`, `latency/1000`, `cost/1`) keep the typical
    /// on-device ranges inside the sensitive part of the curve.
    pub fn score(&self, w: &Weights) -> f64 {
        let energy_term = 1.0 / (1.0 + self.energy_mj.max(0.0) / 1000.0);
        let latency_term = 1.0 / (1.0 + self.latency_ms.max(0.0) / 1000.0);
        let cost_term = 1.0 / (1.0 + self.cost_usd.max(0.0));
        let accuracy_term = self.accuracy.clamp(0.0, 1.0);
        w.energy * energy_term
            + w.latency * latency_term
            + w.cost * cost_term
            + w.accuracy * accuracy_term
    }
}

/// Which axis a percentile query targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Energy,
    Latency,
    Cost,
    Accuracy,
}

/// Accumulates a fleet of [`RunMetrics`] for aggregate analysis.
#[derive(Default)]
pub struct Evaluator {
    samples: Vec<RunMetrics>,
}

impl Evaluator {
    /// An empty evaluator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a run.
    pub fn push(&mut self, m: RunMetrics) {
        self.samples.push(m);
    }

    /// Number of runs recorded.
    pub fn count(&self) -> usize {
        self.samples.len()
    }

    /// All recorded samples.
    pub fn samples(&self) -> &[RunMetrics] {
        &self.samples
    }

    /// Field-wise arithmetic mean, or `None` if empty.
    pub fn mean(&self) -> Option<RunMetrics> {
        if self.samples.is_empty() {
            return None;
        }
        let n = self.samples.len() as f64;
        let mut acc = RunMetrics { energy_mj: 0.0, latency_ms: 0.0, cost_usd: 0.0, accuracy: 0.0 };
        for s in &self.samples {
            acc.energy_mj += s.energy_mj;
            acc.latency_ms += s.latency_ms;
            acc.cost_usd += s.cost_usd;
            acc.accuracy += s.accuracy;
        }
        Some(RunMetrics {
            energy_mj: acc.energy_mj / n,
            latency_ms: acc.latency_ms / n,
            cost_usd: acc.cost_usd / n,
            accuracy: acc.accuracy / n,
        })
    }

    fn extract(&self, metric: Metric) -> Vec<f64> {
        self.samples
            .iter()
            .map(|s| match metric {
                Metric::Energy => s.energy_mj,
                Metric::Latency => s.latency_ms,
                Metric::Cost => s.cost_usd,
                Metric::Accuracy => s.accuracy,
            })
            .collect()
    }

    /// Nearest-rank percentile (`p` in `0..=100`) of one axis. Returns `0.0`
    /// for an empty evaluator.
    pub fn percentile(&self, metric: Metric, p: f64) -> f64 {
        let mut values = self.extract(metric);
        if values.is_empty() {
            return 0.0;
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p = p.clamp(0.0, 100.0);
        let rank = (p / 100.0 * values.len() as f64).ceil().max(1.0) as usize;
        values[rank - 1]
    }

    /// `(index, score)` of the highest-scoring run under `weights`, or `None`.
    pub fn best(&self, weights: &Weights) -> Option<(usize, f64)> {
        self.samples
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.score(weights)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_of_empty_is_zero() {
        assert_eq!(Evaluator::new().percentile(Metric::Cost, 50.0), 0.0);
    }

    #[test]
    fn balanced_weights_sum_to_one() {
        let w = Weights::balanced();
        assert!((w.energy + w.latency + w.cost + w.accuracy - 1.0).abs() < 1e-9);
    }
}
