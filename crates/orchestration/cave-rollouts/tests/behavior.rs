// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for `cave-rollouts` pure-logic surface.
//!
//! Upstream parity target: argoproj/argo-rollouts v1.9.0
//! (<https://github.com/argoproj/argo-rollouts/tree/v1.9.0>).
//!
//! These tests exercise already-implemented, public cave functions that lacked a
//! direct behavioral test (the engine state-machine for canary / blue-green, the
//! string-condition metric evaluator, and the `types` value predicates). They are
//! a pure-logic port: ReplicaSet reconciliation, informers, CRD webhooks and
//! metric-provider clients are scope-cut and not covered here.

use cave_rollouts::engine::{
    advance_blue_green, advance_canary, apply_canary_action, evaluate_metric, initial_status,
    EngineDecision,
};
use cave_rollouts::models::{
    AnalysisPhase, AnalysisRun, BlueGreenStrategy, CanaryStep, CanaryStrategy as MCanaryStrategy,
    RolloutAction, RolloutPhase as MRolloutPhase, RolloutStatus, RolloutStrategy,
};
use cave_rollouts::types::{
    CanaryStrategy as TCanaryStrategy, Experiment, ExperimentVariant, MetricCondition,
    RolloutPhase as TRolloutPhase, RolloutStep,
};
use chrono::Utc;
use uuid::Uuid;

// ── helpers (models-side engine inputs) ───────────────────────────────────────

fn canary_strategy() -> MCanaryStrategy {
    MCanaryStrategy {
        steps: vec![
            CanaryStep::SetWeight { weight: 10 },
            CanaryStep::Pause {
                duration_seconds: Some(60),
            },
            CanaryStep::SetWeight { weight: 90 },
            CanaryStep::SetMirrorWeight { percentage: 25 },
        ],
        stable_service: "app-stable".to_string(),
        canary_service: "app-canary".to_string(),
        max_weight: 50,
        step_weight_increment: 10,
        threshold: None,
        max_analysis_failures: None,
        mirror_percentage: None,
    }
}

fn progressing_status() -> RolloutStatus {
    let mut s = RolloutStatus::default();
    s.phase = MRolloutPhase::Progressing;
    s
}

fn analysis_run(phase: AnalysisPhase) -> AnalysisRun {
    AnalysisRun {
        id: Uuid::new_v4(),
        rollout_id: Uuid::new_v4(),
        template_name: "success-rate".to_string(),
        phase,
        metrics: vec![],
        args: vec![],
        started_at: Utc::now(),
        completed_at: None,
        message: None,
    }
}

fn blue_green_strategy(
    auto_promote_seconds: u64,
    pre_promotion: Option<&str>,
) -> BlueGreenStrategy {
    BlueGreenStrategy {
        active_service: "app-active".to_string(),
        preview_service: "app-preview".to_string(),
        scale_down_delay_seconds: 30,
        auto_promote_seconds,
        pre_promotion_analysis: pre_promotion.map(|s| s.to_string()),
        post_promotion_analysis: None,
        anti_affinity: None,
    }
}

// ── engine::advance_canary ─────────────────────────────────────────────────────

#[test]
fn advance_canary_clamps_step_weight_to_max_weight() {
    // step[2] requests weight 90 but strategy.max_weight is 50 → clamp to 50.
    let strategy = canary_strategy();
    let mut status = progressing_status();
    status.current_step_index = Some(2);

    let decision = advance_canary(&strategy, &mut status, None);

    assert_eq!(decision, EngineDecision::SetWeight(50));
    assert_eq!(status.canary_weight, 50);
    assert_eq!(status.current_step_index, Some(3));
}

#[test]
fn advance_canary_mirror_step_leaves_canary_weight_unchanged() {
    // step[3] is SetMirrorWeight; canary weight must not change, only step advances.
    let strategy = canary_strategy();
    let mut status = progressing_status();
    status.current_step_index = Some(3);
    status.canary_weight = 42;

    let decision = advance_canary(&strategy, &mut status, None);

    assert_eq!(decision, EngineDecision::SetWeight(42));
    assert_eq!(status.canary_weight, 42);
    assert_eq!(status.current_step_index, Some(4));
}

#[test]
fn advance_canary_aborted_phase_short_circuits_to_noop() {
    let strategy = canary_strategy();
    let mut status = progressing_status();
    status.phase = MRolloutPhase::Aborted;
    status.current_step_index = Some(0);

    let decision = advance_canary(&strategy, &mut status, None);

    assert_eq!(decision, EngineDecision::NoOp);
    // No step processing happened.
    assert_eq!(status.current_step_index, Some(0));
    assert_eq!(status.phase, MRolloutPhase::Aborted);
}

#[test]
fn advance_canary_paused_phase_returns_pause_without_advancing() {
    let strategy = canary_strategy();
    let mut status = progressing_status();
    status.phase = MRolloutPhase::Paused;
    status.current_step_index = Some(0);

    let decision = advance_canary(&strategy, &mut status, None);

    assert_eq!(
        decision,
        EngineDecision::Pause {
            duration_seconds: None
        }
    );
    assert_eq!(status.current_step_index, Some(0));
}

#[test]
fn advance_canary_running_analysis_waits_with_noop() {
    let strategy = canary_strategy();
    let mut status = progressing_status();
    status.current_step_index = Some(0);
    let run = analysis_run(AnalysisPhase::Running);

    let decision = advance_canary(&strategy, &mut status, Some(&run));

    assert_eq!(decision, EngineDecision::NoOp);
    // Still Progressing, no step consumed while analysis runs.
    assert_eq!(status.phase, MRolloutPhase::Progressing);
    assert_eq!(status.current_step_index, Some(0));
}

// ── engine::apply_canary_action ────────────────────────────────────────────────

#[test]
fn apply_canary_action_covers_all_manual_arms() {
    let strategy = canary_strategy();

    // Promote: advance one step, phase Progressing, decision NoOp.
    let mut s = progressing_status();
    s.current_step_index = Some(1);
    let d = apply_canary_action(&strategy, &mut s, &RolloutAction::Promote);
    assert_eq!(d, EngineDecision::NoOp);
    assert_eq!(s.current_step_index, Some(2));
    assert_eq!(s.phase, MRolloutPhase::Progressing);

    // Abort: phase Aborted, weight 0, Abort decision.
    let mut s = progressing_status();
    s.canary_weight = 30;
    let d = apply_canary_action(&strategy, &mut s, &RolloutAction::Abort);
    assert_eq!(
        d,
        EngineDecision::Abort {
            reason: "manually aborted".to_string()
        }
    );
    assert_eq!(s.phase, MRolloutPhase::Aborted);
    assert_eq!(s.canary_weight, 0);

    // Pause: phase Paused, Pause{None}.
    let mut s = progressing_status();
    let d = apply_canary_action(&strategy, &mut s, &RolloutAction::Pause);
    assert_eq!(
        d,
        EngineDecision::Pause {
            duration_seconds: None
        }
    );
    assert_eq!(s.phase, MRolloutPhase::Paused);

    // Resume: phase Progressing, NoOp.
    let mut s = progressing_status();
    s.phase = MRolloutPhase::Paused;
    let d = apply_canary_action(&strategy, &mut s, &RolloutAction::Resume);
    assert_eq!(d, EngineDecision::NoOp);
    assert_eq!(s.phase, MRolloutPhase::Progressing);

    // Retry: reset to step 0, weight 0, SetWeight(0), Progressing.
    let mut s = progressing_status();
    s.phase = MRolloutPhase::Aborted;
    s.current_step_index = Some(3);
    s.canary_weight = 50;
    let d = apply_canary_action(&strategy, &mut s, &RolloutAction::Retry);
    assert_eq!(d, EngineDecision::SetWeight(0));
    assert_eq!(s.current_step_index, Some(0));
    assert_eq!(s.canary_weight, 0);
    assert_eq!(s.phase, MRolloutPhase::Progressing);
}

// ── engine::advance_blue_green ─────────────────────────────────────────────────

#[test]
fn advance_blue_green_triggers_pre_promotion_analysis() {
    // pre_promotion set, no pre_analysis yet, preview ready, no auto-promote.
    let strategy = blue_green_strategy(0, Some("pre-check"));
    let mut status = progressing_status();

    let decision = advance_blue_green(&strategy, &mut status, None, None, true);

    assert_eq!(
        decision,
        EngineDecision::RunAnalysis {
            template_name: "pre-check".to_string()
        }
    );
}

#[test]
fn advance_blue_green_auto_promotes_to_healthy() {
    // auto_promote_seconds > 0 → Promote + phase Healthy.
    let strategy = blue_green_strategy(30, None);
    let mut status = progressing_status();

    let decision = advance_blue_green(&strategy, &mut status, None, None, true);

    assert_eq!(decision, EngineDecision::Promote);
    assert_eq!(status.phase, MRolloutPhase::Healthy);
}

#[test]
fn advance_blue_green_aborts_on_failed_analysis() {
    let strategy = blue_green_strategy(30, None);
    let mut status = progressing_status();
    let mut run = analysis_run(AnalysisPhase::Failed);
    run.message = Some("error budget burned".to_string());

    let decision = advance_blue_green(&strategy, &mut status, Some(&run), None, true);

    assert_eq!(
        decision,
        EngineDecision::Abort {
            reason: "error budget burned".to_string()
        }
    );
    assert_eq!(status.phase, MRolloutPhase::Aborted);
}

#[test]
fn advance_blue_green_manual_gate_pauses() {
    // No auto-promote and no pending analysis → manual pause.
    let strategy = blue_green_strategy(0, None);
    let mut status = progressing_status();

    let decision = advance_blue_green(&strategy, &mut status, None, None, true);

    assert_eq!(
        decision,
        EngineDecision::Pause {
            duration_seconds: None
        }
    );
    assert_eq!(status.phase, MRolloutPhase::Paused);
}

// ── engine::initial_status ─────────────────────────────────────────────────────

#[test]
fn initial_status_canary_sets_service_weights() {
    let strategy = RolloutStrategy::Canary(canary_strategy());
    let status = initial_status(&strategy);

    assert_eq!(status.phase, MRolloutPhase::Progressing);
    let canary = status.canary.expect("canary status present");
    assert_eq!(canary.weights.stable.service_name, "app-stable");
    assert_eq!(canary.weights.stable.weight, 100);
    assert_eq!(canary.weights.canary.service_name, "app-canary");
    assert_eq!(canary.weights.canary.weight, 0);
    assert_eq!(canary.current_step_index, Some(0));
    assert!(status.blue_green.is_none());
}

#[test]
fn initial_status_blue_green_sets_active_and_preview() {
    let strategy = RolloutStrategy::BlueGreen(blue_green_strategy(0, None));
    let status = initial_status(&strategy);

    assert_eq!(status.phase, MRolloutPhase::Progressing);
    let bg = status.blue_green.expect("blue/green status present");
    assert_eq!(bg.active_rs.as_deref(), Some("app-active"));
    assert_eq!(bg.preview_rs.as_deref(), Some("app-preview"));
    assert!(status.canary.is_none());
}

// ── engine::evaluate_metric (string-condition entrypoint) ──────────────────────

#[test]
fn evaluate_metric_failure_condition_respects_limit() {
    // failure_condition ">= 0.10": two values breach (0.12, 0.15), limit 1 → Failed.
    let phase = evaluate_metric(
        "error-rate",
        &[0.05, 0.12, 0.15],
        1,
        None,
        Some(">= 0.10"),
    );
    assert_eq!(phase, AnalysisPhase::Failed);

    // Same two breaches but limit raised to 2 → not greater than limit → Successful.
    let phase = evaluate_metric(
        "error-rate",
        &[0.05, 0.12, 0.15],
        2,
        None,
        Some(">= 0.10"),
    );
    assert_eq!(phase, AnalysisPhase::Successful);
}

#[test]
fn evaluate_metric_empty_values_is_inconclusive() {
    let phase = evaluate_metric("latency", &[], 0, Some("< 100"), None);
    assert_eq!(phase, AnalysisPhase::Inconclusive);
}

// ── types::MetricCondition::evaluate ───────────────────────────────────────────

#[test]
fn metric_condition_evaluate_boundaries() {
    // GreaterThan is strict.
    let gt = MetricCondition::GreaterThan { threshold: 0.95 };
    assert!(gt.evaluate(0.96));
    assert!(!gt.evaluate(0.95));
    assert!(!gt.evaluate(0.94));

    // LessThan is strict.
    let lt = MetricCondition::LessThan { threshold: 100.0 };
    assert!(lt.evaluate(99.9));
    assert!(!lt.evaluate(100.0));

    // Between is inclusive on both bounds.
    let between = MetricCondition::Between { lo: 1.0, hi: 5.0 };
    assert!(between.evaluate(1.0));
    assert!(between.evaluate(5.0));
    assert!(between.evaluate(3.0));
    assert!(!between.evaluate(0.99));
    assert!(!between.evaluate(5.01));
}

// ── types::CanaryStrategy::weight_at_step ──────────────────────────────────────

#[test]
fn canary_strategy_weight_at_step() {
    let strategy = TCanaryStrategy {
        steps: vec![
            RolloutStep::SetWeight { weight: 20 },
            RolloutStep::Pause {
                duration_seconds: Some(30),
            },
            RolloutStep::SetWeight { weight: 60 },
        ],
        analysis_template: None,
        max_surge: 25,
    };

    // SetWeight steps return their weight.
    assert_eq!(strategy.weight_at_step(0), Some(20));
    assert_eq!(strategy.weight_at_step(2), Some(60));
    // Non-weight step → None.
    assert_eq!(strategy.weight_at_step(1), None);
    // Out-of-range index → None.
    assert_eq!(strategy.weight_at_step(3), None);
}

// ── types::RolloutPhase::is_terminal ───────────────────────────────────────────

#[test]
fn rollout_phase_is_terminal() {
    assert!(TRolloutPhase::Healthy.is_terminal());
    assert!(TRolloutPhase::Degraded.is_terminal());
    assert!(TRolloutPhase::Error.is_terminal());

    assert!(!TRolloutPhase::Pending.is_terminal());
    assert!(!TRolloutPhase::Progressing.is_terminal());
    assert!(!TRolloutPhase::Paused.is_terminal());
}

// ── types::Experiment::total_weight ────────────────────────────────────────────

#[test]
fn experiment_total_weight_sums_variants() {
    let variant = |name: &str, weight: u8| ExperimentVariant {
        name: name.to_string(),
        template_name: "tmpl".to_string(),
        replicas: 2,
        weight,
    };
    let exp = Experiment::new(
        "ab-test",
        "default",
        vec![variant("a", 25), variant("b", 35), variant("c", 40)],
    );
    assert_eq!(exp.total_weight(), 100);

    // Empty variant list sums to 0.
    let empty = Experiment::new("none", "default", vec![]);
    assert_eq!(empty.total_weight(), 0);
}
