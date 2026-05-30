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

use cave_oncall::engine::{
    num_alerts_condition_met, num_alerts_in_window, within_notify_window, EscalationRun,
    MAX_TIMES_REPEAT,
};
use cave_oncall::models::{EscalationStep, EscalationStepType};
use chrono::{Duration, TimeZone, Utc};

/// Minutes-since-midnight helper for readable window tests.
fn hm(h: u32, m: u32) -> u32 {
    h * 60 + m
}

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

// ---------------------------------------------------------------------------
// Cycle 2 — STEP_NOTIFY_IF_TIME window membership (utils.py eta logic)
// ---------------------------------------------------------------------------
//
// `eta_for_escalation_step_notify_if_time` returns None (escalation proceeds
// immediately) exactly when "now" is inside the [from, to) window; otherwise it
// schedules a future ETA (escalation pauses). `within_notify_window` is that
// membership predicate, with all three upstream branches.

#[test]
fn notify_window_normal_daytime_window() {
    // 09:00 <= now < 17:00 proceeds; the boundaries follow [from, to).
    let (from, to) = (hm(9, 0), hm(17, 0));
    assert!(!within_notify_window(from, to, hm(8, 59)));
    assert!(within_notify_window(from, to, hm(9, 0))); // inclusive start
    assert!(within_notify_window(from, to, hm(12, 30)));
    assert!(within_notify_window(from, to, hm(16, 59)));
    assert!(!within_notify_window(from, to, hm(17, 0))); // exclusive end
    assert!(!within_notify_window(from, to, hm(23, 0)));
}

#[test]
fn notify_window_overnight_wraps_past_midnight() {
    // 22:00 -> 06:00 overnight window: in-window when now >= 22:00 OR now < 06:00.
    let (from, to) = (hm(22, 0), hm(6, 0));
    assert!(within_notify_window(from, to, hm(23, 30)));
    assert!(within_notify_window(from, to, hm(22, 0))); // inclusive start
    assert!(within_notify_window(from, to, hm(0, 0)));
    assert!(within_notify_window(from, to, hm(5, 59)));
    assert!(!within_notify_window(from, to, hm(6, 0))); // exclusive end
    assert!(!within_notify_window(from, to, hm(12, 0))); // dead zone
    assert!(!within_notify_window(from, to, hm(21, 59)));
}

#[test]
fn notify_window_degenerate_equal_bounds_only_matches_the_instant() {
    // from == to is a point window: proceeds only exactly at that minute.
    let t = hm(10, 0);
    assert!(within_notify_window(t, t, hm(10, 0)));
    assert!(!within_notify_window(t, t, hm(9, 59)));
    assert!(!within_notify_window(t, t, hm(10, 1)));
}

#[test]
fn notify_if_time_step_type_round_trips() {
    let step = EscalationStepType::NotifyIfTime {
        from_minute: hm(9, 0),
        to_minute: hm(17, 0),
    };
    let json = serde_json::to_string(&step).unwrap();
    let back: EscalationStepType = serde_json::from_str(&json).unwrap();
    assert_eq!(step, back);
}

// ---------------------------------------------------------------------------
// Cycle 3 — STEP_NOTIFY_IF_NUM_ALERTS_IN_TIME_WINDOW
// ---------------------------------------------------------------------------
//
// escalation_policy_snapshot.py counts the alert group's alerts whose
// created_at >= last_alert.created_at - num_minutes_in_window, then:
//     if num_alerts_in_window <= self.num_alerts_in_window: pause
// i.e. escalation proceeds only when the trailing-window count is STRICTLY
// greater than the configured threshold. The window is anchored on the most
// recent alert, not on wall-clock now.

#[test]
fn num_alerts_in_window_counts_from_latest_alert() {
    let base = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
    // Alerts at t-30, t-9, t-5, t-1, t (minutes relative to the latest).
    let times = vec![
        base - Duration::minutes(30),
        base - Duration::minutes(9),
        base - Duration::minutes(5),
        base - Duration::minutes(1),
        base,
    ];
    // 10-minute trailing window anchored on `base`: includes t-9,t-5,t-1,t = 4.
    assert_eq!(num_alerts_in_window(&times, 10), 4);
    // 60-minute window catches all five.
    assert_eq!(num_alerts_in_window(&times, 60), 5);
    // Empty input has no alerts in any window.
    assert_eq!(num_alerts_in_window(&[], 10), 0);
}

#[test]
fn num_alerts_window_boundary_is_inclusive() {
    let base = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
    // An alert exactly at the window edge (t-10 for a 10-min window) is counted
    // because upstream uses `created_at >= last - delta` (>=, inclusive).
    let times = vec![base - Duration::minutes(10), base];
    assert_eq!(num_alerts_in_window(&times, 10), 2);
    // Just outside the edge (t-11) is excluded.
    let times = vec![base - Duration::minutes(11), base];
    assert_eq!(num_alerts_in_window(&times, 10), 1);
}

#[test]
fn num_alerts_condition_proceeds_only_when_strictly_greater() {
    let base = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
    let times: Vec<_> = (0..5).map(|i| base - Duration::minutes(i)).collect(); // 5 alerts, all within 10m

    // count(5) > threshold(3) => proceed.
    assert!(num_alerts_condition_met(&times, 10, 3));
    // count(5) == threshold(5) => `<=` pauses, so NOT met.
    assert!(!num_alerts_condition_met(&times, 10, 5));
    // count(5) < threshold(6) => not met.
    assert!(!num_alerts_condition_met(&times, 10, 6));
}

#[test]
fn notify_if_num_alerts_step_type_round_trips() {
    let step = EscalationStepType::NotifyIfNumAlertsInWindow {
        num_alerts: 3,
        window_minutes: 10,
    };
    let json = serde_json::to_string(&step).unwrap();
    let back: EscalationStepType = serde_json::from_str(&json).unwrap();
    assert_eq!(step, back);
}
