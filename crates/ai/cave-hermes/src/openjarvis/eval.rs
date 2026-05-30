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
