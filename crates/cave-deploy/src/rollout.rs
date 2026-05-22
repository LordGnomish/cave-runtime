// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Progressive delivery strategies — canary, blue-green, rolling, rollback.

use crate::models::{
    Rollout, RolloutStatus, RolloutStep, RolloutStrategy, RolloutStepRequest,
};
use chrono::Utc;
use uuid::Uuid;

/// Create a canary rollout, incrementally shifting traffic through the
/// supplied steps.
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
        stable_revision: "stable".to_string(),
        canary_revision,
        traffic_weight: initial_weight,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        error: None,
    }
}

/// Create a blue-green rollout: spin up the green stack at 0% traffic,
/// then promote to 100% on operator approval.
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
                analysis: Some("preview".to_string()),
            },
            RolloutStep {
                step_index: 1,
                weight: 100,
                pause_duration_secs: None,
                analysis: Some("promote".to_string()),
            },
        ],
        stable_revision: "stable".to_string(),
        canary_revision,
        traffic_weight: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        error: None,
    }
}

/// Create a rolling update: replace pods in four 25% batches with pauses
/// between each batch.
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
        stable_revision: "stable".to_string(),
        canary_revision,
        traffic_weight: 25,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        error: None,
    }
}

/// Abort a rollout and drain canary traffic back to zero.
pub fn rollback(rollout: &mut Rollout) {
    rollout.status = RolloutStatus::Aborting;
    rollout.traffic_weight = 0;
    rollout.updated_at = Utc::now();
}

/// Promote canary to 100% traffic and mark the rollout complete.
///
/// Returns `true` if the promotion was valid (rollout was Progressing or
/// Paused), `false` otherwise.
pub fn promote_canary(rollout: &mut Rollout) -> bool {
    if rollout.status == RolloutStatus::Paused
        || rollout.status == RolloutStatus::Progressing
    {
        rollout.status = RolloutStatus::Promoting;
        rollout.traffic_weight = 100;
        rollout.stable_revision = rollout.canary_revision.clone();
        rollout.updated_at = Utc::now();
        true
    } else {
        false
    }
}

/// Shift canary traffic to `weight` percent (0–100).
///
/// Automatically marks the rollout complete when weight reaches 100.
pub fn traffic_split(rollout: &mut Rollout, weight: u8) {
    rollout.traffic_weight = weight.min(100);
    rollout.updated_at = Utc::now();
    if rollout.traffic_weight == 100 {
        rollout.status = RolloutStatus::Completed;
        rollout.stable_revision = rollout.canary_revision.clone();
    }
}
