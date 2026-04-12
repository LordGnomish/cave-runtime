//! Rollout progression engine — step processing, promotion, and rollback.

use crate::types::{
    CanaryStrategy, Rollout, RolloutPhase, RolloutStep, RolloutStrategy,
};
use chrono::Utc;

/// Attempt to advance the rollout to the next step.
/// Returns `true` if a step was advanced.
pub fn advance_step(rollout: &mut Rollout) -> bool {
    if rollout.phase != RolloutPhase::Progressing {
        return false;
    }
    let total_steps = step_count(&rollout.strategy);
    if rollout.current_step_index + 1 < total_steps {
        rollout.current_step_index += 1;
        rollout.updated_at = Utc::now();
        true
    } else {
        false
    }
}

/// Promote the rollout: mark canary as stable, phase → Healthy.
pub fn promote(rollout: &mut Rollout) {
    rollout.stable_revision = rollout.canary_revision.clone();
    rollout.phase = RolloutPhase::Healthy;
    rollout.updated_at = Utc::now();
    rollout.message = Some("Rollout promoted to stable".into());
}

/// Rollback: revert canary to stable, phase → Degraded.
pub fn rollback(rollout: &mut Rollout, reason: impl Into<String>) {
    rollout.phase = RolloutPhase::Degraded;
    rollout.updated_at = Utc::now();
    rollout.message = Some(reason.into());
    rollout.current_step_index = 0;
}

/// Pause the rollout (e.g., manual pause or timed pause reached).
pub fn pause(rollout: &mut Rollout) {
    if rollout.phase == RolloutPhase::Progressing {
        rollout.phase = RolloutPhase::Paused;
        rollout.updated_at = Utc::now();
    }
}

/// Resume a paused rollout.
pub fn resume(rollout: &mut Rollout) {
    if rollout.phase == RolloutPhase::Paused {
        rollout.phase = RolloutPhase::Progressing;
        rollout.updated_at = Utc::now();
    }
}

/// Start a new rollout from Pending → Progressing.
pub fn start(rollout: &mut Rollout) {
    if rollout.phase == RolloutPhase::Pending {
        rollout.phase = RolloutPhase::Progressing;
        rollout.updated_at = Utc::now();
    }
}

fn step_count(strategy: &RolloutStrategy) -> usize {
    match strategy {
        RolloutStrategy::Canary(c) => c.steps.len(),
        RolloutStrategy::AbTesting(a) => a.steps.len(),
        RolloutStrategy::BlueGreen(_) => 1,
    }
}

/// Return the current canary traffic weight for a canary rollout (0–100).
/// Returns 0 for non-canary strategies.
pub fn current_canary_weight(rollout: &Rollout) -> u8 {
    match &rollout.strategy {
        RolloutStrategy::Canary(c) => {
            // Walk backward from current_step_index to find the last SetWeight.
            for i in (0..=rollout.current_step_index.min(c.steps.len().saturating_sub(1))).rev() {
                if let Some(RolloutStep::SetWeight { weight }) = c.steps.get(i) {
                    return *weight;
                }
            }
            0
        }
        _ => 0,
    }
}

/// Validate that canary step weights are monotonically increasing and ≤ 100.
pub fn validate_canary_steps(strategy: &CanaryStrategy) -> Result<(), String> {
    let mut last_weight: u8 = 0;
    for (i, step) in strategy.steps.iter().enumerate() {
        if let RolloutStep::SetWeight { weight } = step {
            if *weight > 100 {
                return Err(format!("step {i}: weight {weight} exceeds 100"));
            }
            if *weight < last_weight {
                return Err(format!(
                    "step {i}: weight {weight} decreases from {last_weight}"
                ));
            }
            last_weight = *weight;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AbTestingStrategy, BlueGreenStrategy, RolloutStep, RolloutStrategy};

    fn canary_rollout(steps: Vec<RolloutStep>) -> Rollout {
        Rollout::new(
            "test-rollout",
            "default",
            RolloutStrategy::Canary(CanaryStrategy {
                steps,
                analysis_template: None,
                max_surge: 1,
            }),
            "v1",
            "v2",
        )
    }

    #[test]
    fn test_rollout_initial_phase_is_pending() {
        let r = canary_rollout(vec![RolloutStep::SetWeight { weight: 20 }]);
        assert_eq!(r.phase, RolloutPhase::Pending);
        assert_eq!(r.current_step_index, 0);
    }

    #[test]
    fn test_start_transitions_to_progressing() {
        let mut r = canary_rollout(vec![RolloutStep::SetWeight { weight: 20 }]);
        start(&mut r);
        assert_eq!(r.phase, RolloutPhase::Progressing);
    }

    #[test]
    fn test_advance_step_increments_index() {
        let mut r = canary_rollout(vec![
            RolloutStep::SetWeight { weight: 20 },
            RolloutStep::SetWeight { weight: 50 },
            RolloutStep::SetWeight { weight: 100 },
        ]);
        start(&mut r);
        assert!(advance_step(&mut r));
        assert_eq!(r.current_step_index, 1);
    }

    #[test]
    fn test_advance_step_stops_at_last() {
        let mut r = canary_rollout(vec![RolloutStep::SetWeight { weight: 100 }]);
        start(&mut r);
        assert!(!advance_step(&mut r)); // already at last step
    }

    #[test]
    fn test_promote_sets_healthy_and_stable() {
        let mut r = canary_rollout(vec![RolloutStep::SetWeight { weight: 100 }]);
        start(&mut r);
        promote(&mut r);
        assert_eq!(r.phase, RolloutPhase::Healthy);
        assert_eq!(r.stable_revision, "v2");
    }

    #[test]
    fn test_rollback_sets_degraded_and_resets_step() {
        let mut r = canary_rollout(vec![
            RolloutStep::SetWeight { weight: 20 },
            RolloutStep::SetWeight { weight: 50 },
        ]);
        start(&mut r);
        advance_step(&mut r);
        rollback(&mut r, "metric threshold breached");
        assert_eq!(r.phase, RolloutPhase::Degraded);
        assert_eq!(r.current_step_index, 0);
        assert!(r.message.unwrap().contains("metric threshold breached"));
    }

    #[test]
    fn test_pause_and_resume() {
        let mut r = canary_rollout(vec![RolloutStep::SetWeight { weight: 20 }]);
        start(&mut r);
        pause(&mut r);
        assert_eq!(r.phase, RolloutPhase::Paused);
        resume(&mut r);
        assert_eq!(r.phase, RolloutPhase::Progressing);
    }

    #[test]
    fn test_current_canary_weight() {
        let mut r = canary_rollout(vec![
            RolloutStep::SetWeight { weight: 10 },
            RolloutStep::Pause { duration_seconds: Some(60) },
            RolloutStep::SetWeight { weight: 50 },
        ]);
        start(&mut r);
        assert_eq!(current_canary_weight(&r), 10);
        advance_step(&mut r); // now at step 1 (pause)
        assert_eq!(current_canary_weight(&r), 10); // last SetWeight was index 0
    }

    #[test]
    fn test_validate_canary_steps_valid() {
        let strategy = CanaryStrategy {
            steps: vec![
                RolloutStep::SetWeight { weight: 10 },
                RolloutStep::SetWeight { weight: 30 },
                RolloutStep::SetWeight { weight: 100 },
            ],
            analysis_template: None,
            max_surge: 1,
        };
        assert!(validate_canary_steps(&strategy).is_ok());
    }

    #[test]
    fn test_validate_canary_steps_weight_exceeds_100() {
        let strategy = CanaryStrategy {
            steps: vec![RolloutStep::SetWeight { weight: 101 }],
            analysis_template: None,
            max_surge: 1,
        };
        assert!(validate_canary_steps(&strategy).is_err());
    }

    #[test]
    fn test_validate_canary_steps_decreasing_weight() {
        let strategy = CanaryStrategy {
            steps: vec![
                RolloutStep::SetWeight { weight: 50 },
                RolloutStep::SetWeight { weight: 20 },
            ],
            analysis_template: None,
            max_surge: 1,
        };
        assert!(validate_canary_steps(&strategy).is_err());
    }

    #[test]
    fn test_blue_green_step_count_is_one() {
        let r = Rollout::new(
            "bg",
            "default",
            RolloutStrategy::BlueGreen(BlueGreenStrategy {
                active_service: "svc-active".into(),
                preview_service: "svc-preview".into(),
                auto_promotion_seconds: Some(30),
                analysis_template: None,
            }),
            "v1",
            "v2",
        );
        assert_eq!(step_count(&r.strategy), 1);
    }
}
