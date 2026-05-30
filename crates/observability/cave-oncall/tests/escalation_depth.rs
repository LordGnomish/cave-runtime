// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Depth ports of grafana/oncall (v1.10.0) escalation-policy execution
//! semantics that the stateless `next_escalation_step` walk did not cover.
//!
//! Upstream references (engine/apps/alerts):
//!   * `models/escalation_policy.py`        — `MAX_TIMES_REPEAT = 5`,
//!     `STEP_NOTIFY_IF_TIME` (from_time/to_time), and
//!     `STEP_NOTIFY_IF_NUM_ALERTS_IN_TIME_WINDOW`
//!     (num_alerts_in_window / num_minutes_in_window).
//!   * `escalation_snapshot/snapshot_classes/escalation_policy_snapshot.py`
//!     — the per-step execution that pauses on the time / num-alerts guards.
//!   * `escalation_snapshot/utils.py::eta_for_escalation_step_notify_if_time`
//!     — the time-window membership test (incl. overnight wrap).

use cave_oncall::engine::{EscalationRun, MAX_TIMES_REPEAT};
use cave_oncall::models::{EscalationStep, EscalationStepType};

fn notify(order: u32) -> EscalationStep {
    EscalationStep {
        order,
        step_type: EscalationStepType::NotifyOnCall,
        timeout_seconds: 60,
    }
}

fn repeat(order: u32) -> EscalationStep {
    EscalationStep {
        order,
        step_type: EscalationStepType::RepeatFromStart,
        timeout_seconds: 0,
    }
}

// ---------------------------------------------------------------------------
// Cycle 1 — bounded RepeatFromStart (MAX_TIMES_REPEAT = 5)
// ---------------------------------------------------------------------------

#[test]
fn max_times_repeat_constant_matches_upstream() {
    // engine/apps/alerts/models/escalation_policy.py: MAX_TIMES_REPEAT = 5
    assert_eq!(MAX_TIMES_REPEAT, 5);
}

#[test]
fn escalation_without_repeat_runs_each_step_once_then_finishes() {
    let steps = vec![notify(0), notify(1), notify(2)];
    let mut run = EscalationRun::new();

    assert_eq!(run.next_step(&steps), Some(0));
    assert_eq!(run.next_step(&steps), Some(1));
    assert_eq!(run.next_step(&steps), Some(2));
    // No repeat step: escalation is finished after the last policy step.
    assert_eq!(run.next_step(&steps), None);
    assert!(run.is_finished());
    assert_eq!(run.repeat_count(), 0);
}

#[test]
fn repeat_from_start_loops_back_to_step_zero() {
    // [notify, repeat] — after firing step 0 the repeat sends us back to 0.
    let steps = vec![notify(0), repeat(1)];
    let mut run = EscalationRun::new();

    // First pass fires the single notify, then the repeat rewinds.
    assert_eq!(run.next_step(&steps), Some(0));
    // The repeat step itself is not "fired"; it transparently rewinds and the
    // next emitted step is step 0 again, with the repeat counter incremented.
    assert_eq!(run.next_step(&steps), Some(0));
    assert_eq!(run.repeat_count(), 1);
}

#[test]
fn repeat_is_capped_at_max_times_repeat_then_finishes() {
    // A two-notify chain that repeats: [notify, notify, repeat].
    // Upstream caps repeats at MAX_TIMES_REPEAT (5). Each pass fires 2 notifies;
    // the initial pass plus 5 repeated passes => 2 + 5*2 = 12 fired steps,
    // then escalation finishes (the 6th repeat encounter is over budget).
    let steps = vec![notify(0), notify(1), repeat(2)];
    let mut run = EscalationRun::new();

    let mut fired = 0usize;
    while let Some(idx) = run.next_step(&steps) {
        // Only real (non-repeat) steps are ever emitted.
        assert!(matches!(steps[idx].step_type, EscalationStepType::NotifyOnCall));
        fired += 1;
        assert!(fired <= 100, "escalation must terminate, not loop forever");
    }

    assert_eq!(fired, 12);
    assert!(run.is_finished());
    assert_eq!(run.repeat_count(), MAX_TIMES_REPEAT);
}
