// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Evaluation harness — OpenJarvis primitive.
//!
//! On-device agents cannot pick a backend on capability alone; the right
//! backend depends on the *budget* — how much energy, latency, and cost a
//! task may spend, and the minimum accuracy it must clear. This module
//! records per-run [`EvalMetrics`], aggregates samples, scores a backend
//! against caller weights, and ranks candidate backends so
//! [`super::backend::BackendRegistry`] selection can be metric-driven
//! rather than static.

use serde::{Deserialize, Serialize};

use super::backend::Backend;

/// One measured run of a backend against a task.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EvalMetrics {
    /// Energy spent, joules. Lower is better.
    pub energy_joules: f64,
    /// Wall-clock latency, milliseconds. Lower is better.
    pub latency_ms: f64,
    /// Marginal cost, USD (0.0 for a local run). Lower is better.
    pub cost_usd: f64,
    /// Task accuracy in `[0, 1]`. Higher is better.
    pub accuracy: f64,
}

impl EvalMetrics {
    pub fn new(energy_joules: f64, latency_ms: f64, cost_usd: f64, accuracy: f64) -> Self {
        Self {
            energy_joules,
            latency_ms,
            cost_usd,
            accuracy,
        }
    }

    /// True when every defined budget cap/floor is satisfied.
    pub fn within_budget(&self, budget: &EvalBudget) -> bool {
        if let Some(cap) = budget.max_energy_joules
            && self.energy_joules > cap
        {
            return false;
        }
        if let Some(cap) = budget.max_latency_ms
            && self.latency_ms > cap
        {
            return false;
        }
        if let Some(cap) = budget.max_cost_usd
            && self.cost_usd > cap
        {
            return false;
        }
        if let Some(floor) = budget.min_accuracy
            && self.accuracy < floor
        {
            return false;
        }
        true
    }

    /// Weighted desirability — higher is better. Accuracy contributes
    /// positively; energy/latency/cost contribute a diminishing penalty
    /// (`w / (1 + value)`) so the score stays bounded and monotone in the
    /// intuitive direction.
    pub fn score(&self, w: &ScoreWeights) -> f64 {
        w.accuracy * self.accuracy
            + w.latency / (1.0 + self.latency_ms)
            + w.energy / (1.0 + self.energy_joules)
            + w.cost / (1.0 + self.cost_usd)
    }
}

/// Per-dimension caps/floors. A `None` field is unconstrained.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct EvalBudget {
    pub max_energy_joules: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub max_cost_usd: Option<f64>,
    pub min_accuracy: Option<f64>,
}

impl EvalBudget {
    pub fn unlimited() -> Self {
        Self::default()
    }

    pub fn max_energy_joules(mut self, v: f64) -> Self {
        self.max_energy_joules = Some(v);
        self
    }

    pub fn max_latency_ms(mut self, v: f64) -> Self {
        self.max_latency_ms = Some(v);
        self
    }

    pub fn max_cost_usd(mut self, v: f64) -> Self {
        self.max_cost_usd = Some(v);
        self
    }

    pub fn min_accuracy(mut self, v: f64) -> Self {
        self.min_accuracy = Some(v);
        self
    }
}

/// Relative importance of each dimension in [`EvalMetrics::score`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScoreWeights {
    pub energy: f64,
    pub latency: f64,
    pub cost: f64,
    pub accuracy: f64,
}

impl Default for ScoreWeights {
    /// Local-first default: accuracy dominates, latency next, then energy,
    /// then cost (cost is usually 0 on-device).
    fn default() -> Self {
        Self {
            energy: 10.0,
            latency: 100.0,
            cost: 1.0,
            accuracy: 1.0,
        }
    }
}

/// Accumulates [`EvalMetrics`] samples and aggregates them.
#[derive(Debug, Default, Clone)]
pub struct EvalHarness {
    samples: Vec<EvalMetrics>,
}

impl EvalHarness {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, m: EvalMetrics) -> &mut Self {
        self.samples.push(m);
        self
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Arithmetic mean across all recorded samples, or `None` if empty.
    pub fn mean(&self) -> Option<EvalMetrics> {
        if self.samples.is_empty() {
            return None;
        }
        let n = self.samples.len() as f64;
        let mut acc = EvalMetrics::new(0.0, 0.0, 0.0, 0.0);
        for s in &self.samples {
            acc.energy_joules += s.energy_joules;
            acc.latency_ms += s.latency_ms;
            acc.cost_usd += s.cost_usd;
            acc.accuracy += s.accuracy;
        }
        Some(EvalMetrics::new(
            acc.energy_joules / n,
            acc.latency_ms / n,
            acc.cost_usd / n,
            acc.accuracy / n,
        ))
    }
}

/// Rank candidate backends by score, dropping any whose metrics fall
/// outside `budget`. Best-first.
pub fn rank_backends(
    entries: &[(Backend, EvalMetrics)],
    budget: &EvalBudget,
    weights: &ScoreWeights,
) -> Vec<(Backend, f64)> {
    let mut scored: Vec<(Backend, f64)> = entries
        .iter()
        .filter(|(_, m)| m.within_budget(budget))
        .map(|(b, m)| (*b, m.score(weights)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openjarvis::backend::Backend;

    fn sample() -> EvalMetrics {
        EvalMetrics::new(12.0, 80.0, 0.0, 0.95)
    }

    #[test]
    fn within_unlimited_budget_always_true() {
        assert!(sample().within_budget(&EvalBudget::unlimited()));
    }

    #[test]
    fn latency_over_cap_is_out_of_budget() {
        let b = EvalBudget::unlimited().max_latency_ms(50.0);
        assert!(!sample().within_budget(&b));
    }

    #[test]
    fn accuracy_below_floor_is_out_of_budget() {
        let b = EvalBudget::unlimited().min_accuracy(0.99);
        assert!(!sample().within_budget(&b));
    }

    #[test]
    fn energy_and_cost_caps_enforced() {
        let b = EvalBudget::unlimited().max_energy_joules(10.0);
        assert!(!sample().within_budget(&b));
        let b2 = EvalBudget::unlimited().max_cost_usd(0.0);
        assert!(sample().within_budget(&b2), "zero-cost local run fits a $0 cap");
    }

    #[test]
    fn harness_mean_aggregates_samples() {
        let mut h = EvalHarness::new();
        h.record(EvalMetrics::new(10.0, 100.0, 0.0, 0.9));
        h.record(EvalMetrics::new(20.0, 200.0, 0.0, 1.0));
        assert_eq!(h.len(), 2);
        let m = h.mean().unwrap();
        assert!((m.latency_ms - 150.0).abs() < 1e-9);
        assert!((m.energy_joules - 15.0).abs() < 1e-9);
        assert!((m.accuracy - 0.95).abs() < 1e-9);
    }

    #[test]
    fn empty_harness_has_no_mean() {
        assert!(EvalHarness::new().mean().is_none());
    }

    #[test]
    fn lower_latency_scores_higher() {
        let w = ScoreWeights::default();
        let fast = EvalMetrics::new(10.0, 50.0, 0.0, 0.9);
        let slow = EvalMetrics::new(10.0, 500.0, 0.0, 0.9);
        assert!(fast.score(&w) > slow.score(&w));
    }

    #[test]
    fn higher_accuracy_scores_higher() {
        let w = ScoreWeights::default();
        let good = EvalMetrics::new(10.0, 100.0, 0.0, 0.99);
        let bad = EvalMetrics::new(10.0, 100.0, 0.0, 0.50);
        assert!(good.score(&w) > bad.score(&w));
    }

    #[test]
    fn rank_filters_out_of_budget_and_orders_best_first() {
        let budget = EvalBudget::unlimited().max_latency_ms(300.0);
        let w = ScoreWeights::default();
        let entries = vec![
            (Backend::Ollama, EvalMetrics::new(10.0, 250.0, 0.0, 0.90)),
            (Backend::Mlx, EvalMetrics::new(8.0, 90.0, 0.0, 0.92)),
            (Backend::Vllm, EvalMetrics::new(40.0, 600.0, 0.0, 0.99)), // over latency cap
        ];
        let ranked = rank_backends(&entries, &budget, &w);
        assert_eq!(ranked.len(), 2, "vLLM excluded by latency budget");
        assert_eq!(ranked[0].0, Backend::Mlx, "fastest in-budget wins");
    }
}
