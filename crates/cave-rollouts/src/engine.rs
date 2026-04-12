//! Rollout state-machine engine.
//!
//! Determines what action to take next for a given rollout given its current
//! status, the active strategy, and the latest analysis results.

use crate::models::{
    AnalysisPhase, AnalysisRun, BlueGreenStrategy, BlueGreenStatus, CanaryStatus, CanaryStep,
    CanaryStrategy, RolloutAction, RolloutPhase, RolloutStatus, RolloutStrategy, TrafficWeights,
    WeightDestination,
};
use chrono::Utc;
use tracing::{debug, info, warn};

// ── Engine decision types ─────────────────────────────────────────────────────

/// What the engine wants the controller to do next.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineDecision {
    /// Traffic split is correct; do nothing.
    NoOp,
    /// Shift canary weight to `weight` percent.
    SetWeight(u8),
    /// Pause and wait for manual gate or duration.
    Pause { duration_seconds: Option<u64> },
    /// Launch an analysis run for the given template.
    RunAnalysis { template_name: String },
    /// Promote: cut over fully to the new version.
    Promote,
    /// Abort: roll back to stable.
    Abort { reason: String },
    /// Send a notification.
    Notify { message: String },
}

// ── Canary engine ─────────────────────────────────────────────────────────────

/// Advance a canary rollout by one tick.
pub fn advance_canary(
    strategy: &CanaryStrategy,
    status: &mut RolloutStatus,
    latest_analysis: Option<&AnalysisRun>,
) -> EngineDecision {
    if status.phase == RolloutPhase::Aborted {
        return EngineDecision::NoOp;
    }
    if status.phase == RolloutPhase::Healthy {
        return EngineDecision::NoOp;
    }
    if status.phase == RolloutPhase::Paused {
        return EngineDecision::Pause { duration_seconds: None };
    }

    // Check if the current analysis failed → abort
    if let Some(run) = latest_analysis {
        if run.phase == AnalysisPhase::Failed || run.phase == AnalysisPhase::Error {
            status.phase = RolloutPhase::Aborted;
            status.message = run.message.clone().or_else(|| Some("analysis failed".to_string()));
            warn!(rollout_name = "?", "analysis failed, aborting canary");
            return EngineDecision::Abort {
                reason: run
                    .message
                    .clone()
                    .unwrap_or_else(|| "analysis failed".to_string()),
            };
        }
        // Still running → wait
        if run.phase == AnalysisPhase::Running || run.phase == AnalysisPhase::Pending {
            return EngineDecision::NoOp;
        }
    }

    let step_idx = status.current_step_index.unwrap_or(0) as usize;

    if step_idx >= strategy.steps.len() {
        // All steps complete — promote
        status.phase = RolloutPhase::Healthy;
        status.canary_weight = 0;
        info!("canary rollout complete, promoting");
        return EngineDecision::Promote;
    }

    let step = &strategy.steps[step_idx];
    debug!(step_idx, "processing canary step");

    match step {
        CanaryStep::SetWeight { weight } => {
            let w = (*weight).min(strategy.max_weight);
            status.canary_weight = w;
            status.current_step_index = Some((step_idx + 1) as u32);
            if let Some(cs) = &mut status.canary {
                cs.weights.canary.weight = w;
                cs.weights.stable.weight = 100 - w;
                cs.current_step_index = Some((step_idx + 1) as u32);
            }
            EngineDecision::SetWeight(w)
        }

        CanaryStep::Pause { duration_seconds } => {
            status.phase = RolloutPhase::Paused;
            status.current_step_index = Some((step_idx + 1) as u32);
            EngineDecision::Pause {
                duration_seconds: *duration_seconds,
            }
        }

        CanaryStep::Analysis { template_name } => {
            status.current_step_index = Some((step_idx + 1) as u32);
            EngineDecision::RunAnalysis {
                template_name: template_name.clone(),
            }
        }

        CanaryStep::SetMirrorWeight { percentage } => {
            status.current_step_index = Some((step_idx + 1) as u32);
            // Mirror doesn't count toward canary weight
            EngineDecision::SetWeight(status.canary_weight)
        }
    }
}

/// Apply a manual action (promote / abort / pause / resume) to a canary rollout.
pub fn apply_canary_action(
    strategy: &CanaryStrategy,
    status: &mut RolloutStatus,
    action: &RolloutAction,
) -> EngineDecision {
    match action {
        RolloutAction::Promote => {
            // Advance one step
            let idx = status.current_step_index.unwrap_or(0);
            status.current_step_index = Some(idx + 1);
            status.phase = RolloutPhase::Progressing;
            EngineDecision::NoOp
        }
        RolloutAction::PromoteFull => {
            status.current_step_index = Some(strategy.steps.len() as u32);
            status.canary_weight = 100;
            status.phase = RolloutPhase::Progressing;
            EngineDecision::Promote
        }
        RolloutAction::Abort => {
            status.phase = RolloutPhase::Aborted;
            status.canary_weight = 0;
            EngineDecision::Abort {
                reason: "manually aborted".to_string(),
            }
        }
        RolloutAction::Pause => {
            status.phase = RolloutPhase::Paused;
            EngineDecision::Pause { duration_seconds: None }
        }
        RolloutAction::Resume => {
            status.phase = RolloutPhase::Progressing;
            EngineDecision::NoOp
        }
        RolloutAction::Retry => {
            status.phase = RolloutPhase::Progressing;
            status.current_step_index = Some(0);
            status.canary_weight = 0;
            EngineDecision::SetWeight(0)
        }
    }
}

// ── Blue/Green engine ─────────────────────────────────────────────────────────

pub fn advance_blue_green(
    strategy: &BlueGreenStrategy,
    status: &mut RolloutStatus,
    pre_analysis: Option<&AnalysisRun>,
    post_analysis: Option<&AnalysisRun>,
    preview_ready: bool,
) -> EngineDecision {
    if status.phase == RolloutPhase::Healthy {
        return EngineDecision::NoOp;
    }
    if status.phase == RolloutPhase::Aborted {
        return EngineDecision::NoOp;
    }

    // Check analysis failures
    for analysis in [pre_analysis, post_analysis].into_iter().flatten() {
        if analysis.phase == AnalysisPhase::Failed || analysis.phase == AnalysisPhase::Error {
            status.phase = RolloutPhase::Aborted;
            return EngineDecision::Abort {
                reason: analysis
                    .message
                    .clone()
                    .unwrap_or_else(|| "pre/post analysis failed".to_string()),
            };
        }
        if analysis.phase == AnalysisPhase::Running || analysis.phase == AnalysisPhase::Pending {
            return EngineDecision::NoOp;
        }
    }

    let bg = status.blue_green.get_or_insert_with(|| BlueGreenStatus {
        active_rs: None,
        preview_rs: None,
        pre_promotion_analysis_run: None,
        post_promotion_analysis_run: None,
        scale_down_delay_start_time: None,
    });

    // Pre-promotion analysis
    if strategy.pre_promotion_analysis.is_some() && pre_analysis.is_none() && preview_ready {
        return EngineDecision::RunAnalysis {
            template_name: strategy
                .pre_promotion_analysis
                .clone()
                .unwrap(),
        };
    }

    // Auto-promote if configured
    if strategy.auto_promote_seconds > 0 {
        info!("blue/green auto-promoting");
        status.phase = RolloutPhase::Healthy;
        return EngineDecision::Promote;
    }

    // Wait for manual promotion
    if status.phase != RolloutPhase::Paused {
        status.phase = RolloutPhase::Paused;
    }
    EngineDecision::Pause { duration_seconds: None }
}

// ── Analysis evaluation ───────────────────────────────────────────────────────

/// Given raw metric values for a single MetricResult, decide whether the
/// analysis succeeds, fails, or is inconclusive.
pub fn evaluate_metric(
    name: &str,
    values: &[f64],
    failure_limit: u32,
    success_condition: Option<&str>,
    failure_condition: Option<&str>,
) -> AnalysisPhase {
    if values.is_empty() {
        return AnalysisPhase::Inconclusive;
    }

    let mut failures = 0u32;
    for &v in values {
        if let Some(cond) = failure_condition {
            if evaluate_simple_condition(v, cond) {
                failures += 1;
            }
        } else if let Some(cond) = success_condition {
            if !evaluate_simple_condition(v, cond) {
                failures += 1;
            }
        }
    }

    if failures > failure_limit {
        AnalysisPhase::Failed
    } else {
        AnalysisPhase::Successful
    }
}

/// Evaluate a simple condition string like "result >= 0.95" or "result < 100".
/// Supports: ==, !=, <, <=, >, >= with numeric RHS.
fn evaluate_simple_condition(value: f64, condition: &str) -> bool {
    let condition = condition.trim().replace("result", "");
    let condition = condition.trim();

    for (op, rest) in [
        (">=", ">="),
        ("<=", "<="),
        ("!=", "!="),
        (">", ">"),
        ("<", "<"),
        ("==", "=="),
    ] {
        if let Some(rhs) = condition.strip_prefix(rest) {
            if let Ok(threshold) = rhs.trim().parse::<f64>() {
                return match op {
                    ">=" => value >= threshold,
                    "<=" => value <= threshold,
                    "!=" => (value - threshold).abs() > f64::EPSILON,
                    ">" => value > threshold,
                    "<" => value < threshold,
                    "==" => (value - threshold).abs() < f64::EPSILON,
                    _ => false,
                };
            }
        }
    }
    false
}

// ── Initial status factory ────────────────────────────────────────────────────

pub fn initial_status(strategy: &RolloutStrategy) -> RolloutStatus {
    let mut status = RolloutStatus::default();
    status.phase = RolloutPhase::Progressing;
    match strategy {
        RolloutStrategy::Canary(s) => {
            status.canary = Some(CanaryStatus {
                weights: TrafficWeights {
                    stable: WeightDestination {
                        service_name: s.stable_service.clone(),
                        weight: 100,
                        pod_template_hash: None,
                    },
                    canary: WeightDestination {
                        service_name: s.canary_service.clone(),
                        weight: 0,
                        pod_template_hash: None,
                    },
                    additional: vec![],
                },
                current_step_analysis_run: None,
                current_background_analysis_run: None,
                current_step_index: Some(0),
            });
        }
        RolloutStrategy::BlueGreen(s) => {
            status.blue_green = Some(BlueGreenStatus {
                active_rs: Some(s.active_service.clone()),
                preview_rs: Some(s.preview_service.clone()),
                pre_promotion_analysis_run: None,
                post_promotion_analysis_run: None,
                scale_down_delay_start_time: None,
            });
        }
        RolloutStrategy::ABTest(_) => {}
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    fn canary_strategy_3_steps() -> CanaryStrategy {
        CanaryStrategy {
            steps: vec![
                CanaryStep::SetWeight { weight: 10 },
                CanaryStep::Pause { duration_seconds: Some(60) },
                CanaryStep::SetWeight { weight: 50 },
            ],
            stable_service: "app-stable".to_string(),
            canary_service: "app-canary".to_string(),
            max_weight: 100,
            step_weight_increment: 10,
            threshold: None,
            max_analysis_failures: None,
            mirror_percentage: None,
        }
    }

    fn default_status() -> RolloutStatus {
        let mut s = RolloutStatus::default();
        s.phase = RolloutPhase::Progressing;
        s
    }

    #[test]
    fn canary_first_step_sets_weight() {
        let strategy = canary_strategy_3_steps();
        let mut status = default_status();
        let decision = advance_canary(&strategy, &mut status, None);
        assert_eq!(decision, EngineDecision::SetWeight(10));
        assert_eq!(status.canary_weight, 10);
        assert_eq!(status.current_step_index, Some(1));
    }

    #[test]
    fn canary_second_step_pauses() {
        let strategy = canary_strategy_3_steps();
        let mut status = default_status();
        status.current_step_index = Some(1);
        let decision = advance_canary(&strategy, &mut status, None);
        assert!(matches!(decision, EngineDecision::Pause { duration_seconds: Some(60) }));
        assert_eq!(status.phase, RolloutPhase::Paused);
    }

    #[test]
    fn canary_completed_promotes() {
        let strategy = canary_strategy_3_steps();
        let mut status = default_status();
        status.current_step_index = Some(strategy.steps.len() as u32);
        let decision = advance_canary(&strategy, &mut status, None);
        assert_eq!(decision, EngineDecision::Promote);
        assert_eq!(status.phase, RolloutPhase::Healthy);
    }

    #[test]
    fn failed_analysis_aborts() {
        let strategy = canary_strategy_3_steps();
        let mut status = default_status();
        let run = AnalysisRun {
            id: Uuid::new_v4(),
            rollout_id: Uuid::new_v4(),
            template_name: "success-rate".to_string(),
            phase: AnalysisPhase::Failed,
            metrics: vec![],
            args: vec![],
            started_at: Utc::now(),
            completed_at: None,
            message: Some("error rate too high".to_string()),
        };
        let decision = advance_canary(&strategy, &mut status, Some(&run));
        assert!(matches!(decision, EngineDecision::Abort { .. }));
        assert_eq!(status.phase, RolloutPhase::Aborted);
    }

    #[test]
    fn manual_full_promote() {
        let strategy = canary_strategy_3_steps();
        let mut status = default_status();
        let decision = apply_canary_action(&strategy, &mut status, &RolloutAction::PromoteFull);
        assert_eq!(decision, EngineDecision::Promote);
        assert_eq!(status.canary_weight, 100);
    }

    #[test]
    fn evaluate_simple_success_condition() {
        assert!(evaluate_simple_condition(0.97, ">= 0.95"));
        assert!(!evaluate_simple_condition(0.93, ">= 0.95"));
        assert!(evaluate_simple_condition(5.0, "< 10"));
        assert!(evaluate_simple_condition(10.0, "== 10"));
    }

    #[test]
    fn metric_evaluation_failure() {
        let phase = evaluate_metric("error-rate", &[0.05, 0.12, 0.15], 1, None, Some(">= 0.10"));
        assert_eq!(phase, AnalysisPhase::Failed); // 2 failures > limit 1
    }

    #[test]
    fn metric_evaluation_success() {
        let phase = evaluate_metric("success-rate", &[0.98, 0.99, 0.97], 0, Some(">= 0.95"), None);
        assert_eq!(phase, AnalysisPhase::Successful);
    }
}
