// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Evaluation tools: per-run energy/latency/cost/accuracy scoring and
//! fleet aggregation (mean, nearest-rank percentile, best-by-score).

use cave_agent::eval::{Evaluator, Metric, RunMetrics, Weights};

#[test]
fn perfect_run_scores_one() {
    let m = RunMetrics {
        energy_mj: 0.0,
        latency_ms: 0.0,
        cost_usd: 0.0,
        accuracy: 1.0,
    };
    let s = m.score(&Weights::balanced());
    assert!((s - 1.0).abs() < 1e-9, "expected 1.0, got {s}");
}

#[test]
fn higher_accuracy_scores_higher_all_else_equal() {
    let lo = RunMetrics { energy_mj: 100.0, latency_ms: 200.0, cost_usd: 0.01, accuracy: 0.5 };
    let hi = RunMetrics { accuracy: 0.9, ..lo };
    let w = Weights::balanced();
    assert!(hi.score(&w) > lo.score(&w));
}

#[test]
fn lower_latency_scores_higher_all_else_equal() {
    let slow = RunMetrics { energy_mj: 10.0, latency_ms: 5000.0, cost_usd: 0.01, accuracy: 0.8 };
    let fast = RunMetrics { latency_ms: 50.0, ..slow };
    let w = Weights::balanced();
    assert!(fast.score(&w) > slow.score(&w));
}

#[test]
fn evaluator_mean_is_field_wise() {
    let mut e = Evaluator::new();
    e.push(RunMetrics { energy_mj: 10.0, latency_ms: 100.0, cost_usd: 0.02, accuracy: 0.6 });
    e.push(RunMetrics { energy_mj: 30.0, latency_ms: 300.0, cost_usd: 0.04, accuracy: 0.8 });
    let mean = e.mean().unwrap();
    assert_eq!(mean.energy_mj, 20.0);
    assert_eq!(mean.latency_ms, 200.0);
    assert!((mean.cost_usd - 0.03).abs() < 1e-9);
    assert!((mean.accuracy - 0.7).abs() < 1e-9);
}

#[test]
fn percentile_uses_nearest_rank() {
    let mut e = Evaluator::new();
    for v in 1..=100 {
        e.push(RunMetrics {
            energy_mj: 0.0,
            latency_ms: v as f64,
            cost_usd: 0.0,
            accuracy: 1.0,
        });
    }
    assert_eq!(e.percentile(Metric::Latency, 95.0), 95.0);
    assert_eq!(e.percentile(Metric::Latency, 50.0), 50.0);
    assert_eq!(e.percentile(Metric::Latency, 100.0), 100.0);
}

#[test]
fn best_returns_highest_scoring_index() {
    let mut e = Evaluator::new();
    e.push(RunMetrics { energy_mj: 0.0, latency_ms: 0.0, cost_usd: 0.0, accuracy: 0.3 });
    e.push(RunMetrics { energy_mj: 0.0, latency_ms: 0.0, cost_usd: 0.0, accuracy: 0.95 });
    e.push(RunMetrics { energy_mj: 0.0, latency_ms: 0.0, cost_usd: 0.0, accuracy: 0.6 });
    let (idx, score) = e.best(&Weights::balanced()).unwrap();
    assert_eq!(idx, 1);
    assert!(score > 0.9);
}

#[test]
fn empty_evaluator_has_no_mean_or_best() {
    let e = Evaluator::new();
    assert!(e.mean().is_none());
    assert!(e.best(&Weights::balanced()).is_none());
    assert_eq!(e.count(), 0);
}

#[test]
fn weights_can_emphasize_accuracy_only() {
    let w = Weights::accuracy_first();
    // A run with terrible energy/latency/cost but perfect accuracy should still
    // score close to its accuracy weight contribution being dominant.
    let bad_but_accurate =
        RunMetrics { energy_mj: 1e6, latency_ms: 1e6, cost_usd: 1e6, accuracy: 1.0 };
    let good_but_wrong =
        RunMetrics { energy_mj: 0.0, latency_ms: 0.0, cost_usd: 0.0, accuracy: 0.0 };
    assert!(bad_but_accurate.score(&w) > good_but_wrong.score(&w));
}
