// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Progressive delivery strategies — canary, blue-green, rolling, rollback.

use crate::models::{Rollout, RolloutStatus, RolloutStep, RolloutStepRequest, RolloutStrategy};
use chrono::Utc;
use uuid::Uuid;

/// Build a canary rollout. Initial traffic weight is taken from the first step
/// (default 5% when no steps are supplied).
pub fn canary_deploy(
    application_id: Uuid,
    canary_revision: String,
    steps: Vec<RolloutStepRequest>,
) -> Rollout {
    let initial_weight = steps.first().map(|s| s.weight).unwrap_or(5);
    let rollout_steps: Vec<RolloutStep> = steps
        .into_iter()
        .enumerate()
        .map(|(i, s)| RolloutStep {
            step_index: i,
            weight: s.weight,
            pause_duration_secs: s.pause_duration_secs,
            analysis: s.analysis,
        })
        .collect();
    Rollout {
        id: Uuid::new_v4(),
        application_id,
        strategy: RolloutStrategy::Canary,
        status: RolloutStatus::Progressing,
        current_step: 0,
        steps: rollout_steps,
        stable_revision: "stable".into(),
        canary_revision,
        traffic_weight: initial_weight,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        error: None,
    }
}

/// Blue/green: green stack at 0% traffic, then 100% on operator promotion.
pub fn blue_green_deploy(application_id: Uuid, canary_revision: String) -> Rollout {
    Rollout {
        id: Uuid::new_v4(),
        application_id,
        strategy: RolloutStrategy::BlueGreen,
        status: RolloutStatus::Progressing,
        current_step: 0,
        steps: vec![
            RolloutStep {
                step_index: 0,
                weight: 0,
                pause_duration_secs: None,
                analysis: Some("preview".into()),
            },
            RolloutStep {
                step_index: 1,
                weight: 100,
                pause_duration_secs: None,
                analysis: Some("promote".into()),
            },
        ],
        stable_revision: "stable".into(),
        canary_revision,
        traffic_weight: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        error: None,
    }
}

/// Rolling update: 25/50/75/100 in four batches with 30s pauses.
pub fn rolling_update(application_id: Uuid, canary_revision: String) -> Rollout {
    Rollout {
        id: Uuid::new_v4(),
        application_id,
        strategy: RolloutStrategy::Rolling,
        status: RolloutStatus::Progressing,
        current_step: 0,
        steps: vec![
            RolloutStep {
                step_index: 0,
                weight: 25,
                pause_duration_secs: Some(30),
                analysis: None,
            },
            RolloutStep {
                step_index: 1,
                weight: 50,
                pause_duration_secs: Some(30),
                analysis: None,
            },
            RolloutStep {
                step_index: 2,
                weight: 75,
                pause_duration_secs: Some(30),
                analysis: None,
            },
            RolloutStep {
                step_index: 3,
                weight: 100,
                pause_duration_secs: None,
                analysis: None,
            },
        ],
        stable_revision: "stable".into(),
        canary_revision,
        traffic_weight: 25,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        error: None,
    }
}

/// Abort an in-flight rollout and drain traffic back to zero.
pub fn rollback(rollout: &mut Rollout) {
    rollout.status = RolloutStatus::Aborting;
    rollout.traffic_weight = 0;
    rollout.updated_at = Utc::now();
}

/// Promote the canary to 100% traffic. Returns true on success — false when
/// the rollout is not in a promotable state.
pub fn promote_canary(rollout: &mut Rollout) -> bool {
    if rollout.status == RolloutStatus::Paused || rollout.status == RolloutStatus::Progressing {
        rollout.status = RolloutStatus::Promoting;
        rollout.traffic_weight = 100;
        rollout.stable_revision = rollout.canary_revision.clone();
        rollout.updated_at = Utc::now();
        true
    } else {
        false
    }
}

/// Shift traffic to `weight` percent (clamped to 100). Marks the rollout
/// Completed when weight reaches 100.
pub fn traffic_split(rollout: &mut Rollout, weight: u8) {
    rollout.traffic_weight = weight.min(100);
    rollout.updated_at = Utc::now();
    if rollout.traffic_weight == 100 {
        rollout.status = RolloutStatus::Completed;
        rollout.stable_revision = rollout.canary_revision.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canary_initial_weight_from_first_step() {
        let r = canary_deploy(
            Uuid::new_v4(),
            "v2".into(),
            vec![
                RolloutStepRequest {
                    weight: 10,
                    pause_duration_secs: Some(60),
                    analysis: None,
                },
                RolloutStepRequest {
                    weight: 50,
                    pause_duration_secs: Some(60),
                    analysis: None,
                },
            ],
        );
        assert_eq!(r.traffic_weight, 10);
        assert_eq!(r.steps.len(), 2);
        assert_eq!(r.strategy, RolloutStrategy::Canary);
    }

    #[test]
    fn blue_green_starts_at_zero() {
        let r = blue_green_deploy(Uuid::new_v4(), "v2".into());
        assert_eq!(r.traffic_weight, 0);
        assert_eq!(r.steps.len(), 2);
    }

    #[test]
    fn rolling_has_four_steps() {
        let r = rolling_update(Uuid::new_v4(), "v2".into());
        assert_eq!(r.steps.len(), 4);
        assert_eq!(r.steps.last().unwrap().weight, 100);
    }

    #[test]
    fn rollback_drains_traffic() {
        let mut r = canary_deploy(Uuid::new_v4(), "v2".into(), vec![]);
        r.traffic_weight = 50;
        rollback(&mut r);
        assert_eq!(r.traffic_weight, 0);
        assert_eq!(r.status, RolloutStatus::Aborting);
    }

    #[test]
    fn promote_canary_only_when_progressing_or_paused() {
        let mut r = canary_deploy(Uuid::new_v4(), "v2".into(), vec![]);
        assert!(promote_canary(&mut r));
        assert_eq!(r.traffic_weight, 100);
        assert_eq!(r.stable_revision, "v2");

        // Now status is Promoting — re-promotion is a no-op
        assert!(!promote_canary(&mut r));
    }

    #[test]
    fn traffic_split_completes_at_100() {
        let mut r = canary_deploy(Uuid::new_v4(), "v3".into(), vec![]);
        traffic_split(&mut r, 100);
        assert_eq!(r.traffic_weight, 100);
        assert_eq!(r.status, RolloutStatus::Completed);
        assert_eq!(r.stable_revision, "v3");
    }
}
