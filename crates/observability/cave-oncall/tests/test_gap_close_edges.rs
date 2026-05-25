// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge-case + boundary tests for cave-oncall engine and model serde.
//!
//! Adds focused coverage for:
//!   * `engine::current_oncall` — override-precedence, inactive rotation,
//!     pre-start window, schedule-mismatch filter.
//!   * `engine::upcoming_shifts` — empty-rotation guard, mismatched-schedule
//!     filter, daily/weekly/custom rotation cadences and ordering.
//!   * `engine::dedupe_fingerprint` — resolved alerts are not deduped against;
//!     newest-firing wins is unspecified, but presence-of-match is asserted.
//!   * `engine::evaluate_silences` — outside-window, partial-matcher,
//!     empty-matcher matches everything.
//!   * `engine::next_escalation_step` — exhausted-elapsed, single-step,
//!     out-of-order (>= cumulative_timeout) and current_escalation_step gating.
//!   * `engine::validate_rotation` — minute overflow, valid edge values
//!     (handoff_hour == 23, handoff_minute == 59), and error variants.
//!   * `models` serde round-trip for the discriminated enums
//!     (`Severity`, `AlertState`, `ScheduleType`, `RotationType`,
//!      `EscalationStepType`) — guards against unintentional rename_all changes.

use cave_oncall::engine::{
    OnCallError, current_oncall, dedupe_fingerprint, evaluate_silences, next_escalation_step,
    upcoming_shifts, validate_rotation,
};
use cave_oncall::models::{
    Alert, AlertState, EscalationPolicy, EscalationStep, EscalationStepType, Rotation,
    RotationType, Schedule, ScheduleType, Severity, ShiftOverride, Silence,
};
use chrono::{Duration, TimeZone, Utc};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn schedule_for(id: Uuid, team_id: Uuid) -> Schedule {
    Schedule {
        id,
        team_id,
        name: "S".into(),
        description: None,
        timezone: "UTC".into(),
        schedule_type: ScheduleType::Rotation,
        created_at: Utc::now(),
    }
}

fn rotation_for(schedule_id: Uuid, rotation_type: RotationType, users: Vec<&str>) -> Rotation {
    Rotation {
        id: Uuid::new_v4(),
        schedule_id,
        name: "R".into(),
        users: users.into_iter().map(|s| s.to_string()).collect(),
        start_date: Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap(),
        rotation_type,
        handoff_hour: 9,
        handoff_minute: 0,
        shift_duration_hours: 24,
        active: true,
    }
}

fn firing_alert(fingerprint: &str, labels: HashMap<String, String>) -> Alert {
    Alert {
        id: Uuid::new_v4(),
        team_id: Uuid::new_v4(),
        title: "t".into(),
        severity: Severity::High,
        source: "src".into(),
        fingerprint: fingerprint.into(),
        state: AlertState::Firing,
        labels,
        annotations: HashMap::new(),
        created_at: Utc::now(),
        ack_at: None,
        ack_by: None,
        resolved_at: None,
        escalation_policy_id: None,
        current_escalation_step: 0,
    }
}

// ---------------------------------------------------------------------------
// current_oncall edge cases
// ---------------------------------------------------------------------------

#[test]
fn current_oncall_returns_none_when_before_rotation_start() {
    let sched_id = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let schedule = schedule_for(sched_id, team_id);
    let rotation = rotation_for(sched_id, RotationType::Daily, vec!["alice", "bob"]);

    let before = rotation.start_date - Duration::hours(1);
    assert!(current_oncall(&schedule, &[rotation], &[], before).is_none());
}

#[test]
fn current_oncall_ignores_inactive_rotation() {
    let sched_id = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let schedule = schedule_for(sched_id, team_id);
    let mut rotation = rotation_for(sched_id, RotationType::Daily, vec!["alice"]);
    rotation.active = false;

    let at = rotation.start_date + Duration::hours(1);
    assert!(current_oncall(&schedule, &[rotation], &[], at).is_none());
}

#[test]
fn current_oncall_ignores_rotation_for_different_schedule() {
    let sched_a = Uuid::new_v4();
    let sched_b = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let schedule = schedule_for(sched_a, team_id);
    // Rotation belongs to sched_b — must be filtered out.
    let rotation = rotation_for(sched_b, RotationType::Daily, vec!["alice"]);

    let at = rotation.start_date + Duration::hours(1);
    assert!(current_oncall(&schedule, &[rotation], &[], at).is_none());
}

#[test]
fn current_oncall_override_outperforms_rotation() {
    let sched_id = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let schedule = schedule_for(sched_id, team_id);
    let rotation = rotation_for(sched_id, RotationType::Daily, vec!["alice", "bob"]);

    let at = rotation.start_date + Duration::hours(2);
    let override_shift = ShiftOverride {
        id: Uuid::new_v4(),
        schedule_id: sched_id,
        user: "zoe".into(),
        start: at - Duration::minutes(30),
        end: at + Duration::minutes(30),
        reason: None,
    };

    let result = current_oncall(&schedule, &[rotation], &[override_shift], at).unwrap();
    assert_eq!(result.user, "zoe");
    assert!(result.is_override);
}

#[test]
fn current_oncall_daily_rotation_cycles_users() {
    let sched_id = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let schedule = schedule_for(sched_id, team_id);
    let rotation = rotation_for(sched_id, RotationType::Daily, vec!["alice", "bob", "carol"]);

    // Day 0 → alice, Day 1 → bob, Day 2 → carol, Day 3 → wraps to alice.
    let day0 = rotation.start_date + Duration::hours(1);
    let day1 = rotation.start_date + Duration::days(1) + Duration::hours(1);
    let day2 = rotation.start_date + Duration::days(2) + Duration::hours(1);
    let day3 = rotation.start_date + Duration::days(3) + Duration::hours(1);

    assert_eq!(
        current_oncall(&schedule, &[rotation.clone()], &[], day0)
            .unwrap()
            .user,
        "alice"
    );
    assert_eq!(
        current_oncall(&schedule, &[rotation.clone()], &[], day1)
            .unwrap()
            .user,
        "bob"
    );
    assert_eq!(
        current_oncall(&schedule, &[rotation.clone()], &[], day2)
            .unwrap()
            .user,
        "carol"
    );
    assert_eq!(
        current_oncall(&schedule, &[rotation], &[], day3).unwrap().user,
        "alice"
    );
}

#[test]
fn current_oncall_custom_rotation_uses_shift_duration() {
    let sched_id = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let schedule = schedule_for(sched_id, team_id);
    let mut rotation = rotation_for(sched_id, RotationType::Custom, vec!["alice", "bob"]);
    rotation.shift_duration_hours = 8;

    let t0 = rotation.start_date + Duration::hours(1); // cycle 0 → alice
    let t1 = rotation.start_date + Duration::hours(9); // cycle 1 → bob
    let t2 = rotation.start_date + Duration::hours(17); // cycle 2 → wraps to alice

    assert_eq!(
        current_oncall(&schedule, &[rotation.clone()], &[], t0)
            .unwrap()
            .user,
        "alice"
    );
    assert_eq!(
        current_oncall(&schedule, &[rotation.clone()], &[], t1)
            .unwrap()
            .user,
        "bob"
    );
    assert_eq!(
        current_oncall(&schedule, &[rotation], &[], t2).unwrap().user,
        "alice"
    );
}

// ---------------------------------------------------------------------------
// upcoming_shifts edge cases
// ---------------------------------------------------------------------------

#[test]
fn upcoming_shifts_filters_by_schedule_and_active() {
    let sched_id = Uuid::new_v4();
    let other_sched = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let schedule = schedule_for(sched_id, team_id);

    let r_match = rotation_for(sched_id, RotationType::Daily, vec!["alice"]);
    let r_other = rotation_for(other_sched, RotationType::Daily, vec!["bob"]);
    let mut r_inactive = rotation_for(sched_id, RotationType::Daily, vec!["carol"]);
    r_inactive.active = false;

    let shifts = upcoming_shifts(&schedule, &[r_match, r_other, r_inactive], 3);
    assert!(!shifts.is_empty());
    assert!(shifts.iter().all(|s| s.schedule_id == sched_id));
    assert!(shifts.iter().all(|s| s.user == "alice"));
}

#[test]
fn upcoming_shifts_is_sorted_by_start() {
    let sched_id = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let schedule = schedule_for(sched_id, team_id);
    let rotation = rotation_for(sched_id, RotationType::Daily, vec!["alice", "bob"]);

    let shifts = upcoming_shifts(&schedule, &[rotation], 14);
    let starts: Vec<_> = shifts.iter().map(|s| s.start).collect();
    let mut sorted = starts.clone();
    sorted.sort();
    assert_eq!(starts, sorted, "upcoming_shifts must be sorted by start");
}

#[test]
fn upcoming_shifts_empty_users_short_circuits() {
    let sched_id = Uuid::new_v4();
    let team_id = Uuid::new_v4();
    let schedule = schedule_for(sched_id, team_id);
    let rotation = rotation_for(sched_id, RotationType::Daily, vec![]);

    let shifts = upcoming_shifts(&schedule, &[rotation], 7);
    assert!(shifts.is_empty());
}

// ---------------------------------------------------------------------------
// dedupe_fingerprint edge cases
// ---------------------------------------------------------------------------

#[test]
fn dedupe_fingerprint_ignores_resolved_alerts() {
    let mut a = firing_alert("fp-1", HashMap::new());
    a.state = AlertState::Resolved;
    let existing = vec![a];
    // Resolved alerts should NOT match — a brand-new firing alert with the same
    // fingerprint should be allowed through.
    assert!(dedupe_fingerprint("fp-1", &existing).is_none());
}

#[test]
fn dedupe_fingerprint_matches_acknowledged_alerts() {
    let mut a = firing_alert("fp-2", HashMap::new());
    a.state = AlertState::Acknowledged;
    let id = a.id;
    let existing = vec![a];
    // Acknowledged is not Resolved → still active, should dedupe.
    assert_eq!(dedupe_fingerprint("fp-2", &existing), Some(id));
}

#[test]
fn dedupe_fingerprint_empty_store_returns_none() {
    assert!(dedupe_fingerprint("anything", &[]).is_none());
}

// ---------------------------------------------------------------------------
// evaluate_silences edge cases
// ---------------------------------------------------------------------------

#[test]
fn evaluate_silences_outside_window_does_not_match() {
    let mut labels = HashMap::new();
    labels.insert("env".into(), "prod".into());
    let alert = firing_alert("fp", labels);

    let mut matcher = HashMap::new();
    matcher.insert("env".into(), "prod".into());

    let now = Utc::now();
    let silence = Silence {
        id: Uuid::new_v4(),
        team_id: alert.team_id,
        matcher,
        start: now - Duration::hours(2),
        end: now - Duration::hours(1),
        created_by: "alice".into(),
        reason: None,
    };
    assert!(!evaluate_silences(&alert, &[silence], now));
}

#[test]
fn evaluate_silences_partial_matcher_does_not_match() {
    let mut labels = HashMap::new();
    labels.insert("env".into(), "prod".into());
    let alert = firing_alert("fp", labels);

    let mut matcher = HashMap::new();
    matcher.insert("env".into(), "prod".into());
    matcher.insert("region".into(), "eu".into()); // alert lacks `region`

    let now = Utc::now();
    let silence = Silence {
        id: Uuid::new_v4(),
        team_id: alert.team_id,
        matcher,
        start: now - Duration::hours(1),
        end: now + Duration::hours(1),
        created_by: "alice".into(),
        reason: None,
    };
    assert!(!evaluate_silences(&alert, &[silence], now));
}

#[test]
fn evaluate_silences_empty_matcher_matches_any_alert() {
    let alert = firing_alert("fp", HashMap::new());
    let now = Utc::now();
    let silence = Silence {
        id: Uuid::new_v4(),
        team_id: alert.team_id,
        matcher: HashMap::new(),
        start: now - Duration::minutes(5),
        end: now + Duration::minutes(5),
        created_by: "alice".into(),
        reason: None,
    };
    // Empty matcher → "all matchers must match" is vacuously true.
    assert!(evaluate_silences(&alert, &[silence], now));
}

#[test]
fn evaluate_silences_end_is_exclusive() {
    let alert = firing_alert("fp", HashMap::new());
    let now = Utc::now();
    let silence = Silence {
        id: Uuid::new_v4(),
        team_id: alert.team_id,
        matcher: HashMap::new(),
        start: now - Duration::minutes(5),
        end: now, // exclusive upper bound → at == end should not match
        created_by: "alice".into(),
        reason: None,
    };
    assert!(!evaluate_silences(&alert, &[silence], now));
}

// ---------------------------------------------------------------------------
// next_escalation_step edge cases
// ---------------------------------------------------------------------------

#[test]
fn next_escalation_step_zero_elapsed_returns_step_zero() {
    let policy = EscalationPolicy {
        id: Uuid::new_v4(),
        team_id: Uuid::new_v4(),
        name: "p".into(),
        steps: vec![
            EscalationStep {
                order: 0,
                step_type: EscalationStepType::NotifyOnCall,
                timeout_seconds: 60,
            },
            EscalationStep {
                order: 1,
                step_type: EscalationStepType::NotifyOnCall,
                timeout_seconds: 60,
            },
        ],
        created_at: Utc::now(),
    };
    let alert = firing_alert("fp", HashMap::new());
    let step = next_escalation_step(&alert, &policy, 0).unwrap();
    assert_eq!(step.order, 0);
}

#[test]
fn next_escalation_step_returns_last_when_far_past_all_timeouts() {
    let policy = EscalationPolicy {
        id: Uuid::new_v4(),
        team_id: Uuid::new_v4(),
        name: "p".into(),
        steps: vec![
            EscalationStep {
                order: 0,
                step_type: EscalationStepType::NotifyOnCall,
                timeout_seconds: 30,
            },
            EscalationStep {
                order: 1,
                step_type: EscalationStepType::NotifyOnCall,
                timeout_seconds: 30,
            },
            EscalationStep {
                order: 2,
                step_type: EscalationStepType::RepeatFromStart,
                timeout_seconds: 30,
            },
        ],
        created_at: Utc::now(),
    };
    let alert = firing_alert("fp", HashMap::new());
    let step = next_escalation_step(&alert, &policy, 10_000).unwrap();
    assert_eq!(step.order, 2);
}

#[test]
fn next_escalation_step_gated_by_current_step() {
    let policy = EscalationPolicy {
        id: Uuid::new_v4(),
        team_id: Uuid::new_v4(),
        name: "p".into(),
        steps: vec![
            EscalationStep {
                order: 0,
                step_type: EscalationStepType::NotifyOnCall,
                timeout_seconds: 60,
            },
            EscalationStep {
                order: 1,
                step_type: EscalationStepType::NotifyOnCall,
                timeout_seconds: 60,
            },
        ],
        created_at: Utc::now(),
    };
    let mut alert = firing_alert("fp", HashMap::new());
    // If we mark current_escalation_step beyond all step.order values, the
    // gating predicate (current_escalation_step <= step.order) fails for every
    // step → no step is returned.
    alert.current_escalation_step = 99;
    assert!(next_escalation_step(&alert, &policy, 1_000).is_none());
}

#[test]
fn next_escalation_step_empty_policy_returns_none() {
    let policy = EscalationPolicy {
        id: Uuid::new_v4(),
        team_id: Uuid::new_v4(),
        name: "empty".into(),
        steps: vec![],
        created_at: Utc::now(),
    };
    let alert = firing_alert("fp", HashMap::new());
    assert!(next_escalation_step(&alert, &policy, 0).is_none());
    assert!(next_escalation_step(&alert, &policy, 9_999).is_none());
}

// ---------------------------------------------------------------------------
// validate_rotation edge cases
// ---------------------------------------------------------------------------

#[test]
fn validate_rotation_rejects_invalid_minute() {
    let mut rot = rotation_for(Uuid::new_v4(), RotationType::Daily, vec!["alice"]);
    rot.handoff_minute = 60;
    match validate_rotation(&rot) {
        Err(OnCallError::InvalidRotation(msg)) => assert!(msg.contains("handoff_minute")),
        other => panic!("expected InvalidRotation, got {other:?}"),
    }
}

#[test]
fn validate_rotation_accepts_boundary_values() {
    let mut rot = rotation_for(Uuid::new_v4(), RotationType::Daily, vec!["alice"]);
    rot.handoff_hour = 23;
    rot.handoff_minute = 59;
    rot.shift_duration_hours = 1;
    assert!(validate_rotation(&rot).is_ok());
}

#[test]
fn oncall_error_display_messages_are_distinct() {
    // Defensive: ensure thiserror Display is not accidentally collapsed.
    let messages = [
        OnCallError::InvalidRotation("x".into()).to_string(),
        OnCallError::TeamNotFound.to_string(),
        OnCallError::ScheduleNotFound.to_string(),
        OnCallError::UserNotFound.to_string(),
        OnCallError::AlertNotFound.to_string(),
        OnCallError::InvalidTimeRange.to_string(),
        OnCallError::AlreadyAcknowledged.to_string(),
    ];
    let mut seen = std::collections::HashSet::new();
    for m in &messages {
        assert!(seen.insert(m.clone()), "duplicate error message: {m}");
    }
}

// ---------------------------------------------------------------------------
// Serde round-trip for discriminated enums
// ---------------------------------------------------------------------------

#[test]
fn severity_serde_pascal_case() {
    let s = serde_json::to_string(&Severity::Critical).unwrap();
    assert_eq!(s, "\"Critical\"");
    let back: Severity = serde_json::from_str("\"Info\"").unwrap();
    assert_eq!(back, Severity::Info);
}

#[test]
fn alert_state_serde_pascal_case() {
    let s = serde_json::to_string(&AlertState::Acknowledged).unwrap();
    assert_eq!(s, "\"Acknowledged\"");
    let back: AlertState = serde_json::from_str("\"Resolved\"").unwrap();
    assert_eq!(back, AlertState::Resolved);
}

#[test]
fn schedule_type_serde_snake_case() {
    let s = serde_json::to_string(&ScheduleType::FixedShift).unwrap();
    assert_eq!(s, "\"fixed_shift\"");
    let back: ScheduleType = serde_json::from_str("\"rotation\"").unwrap();
    assert_eq!(back, ScheduleType::Rotation);
}

#[test]
fn rotation_type_serde_snake_case() {
    assert_eq!(
        serde_json::to_string(&RotationType::Weekly).unwrap(),
        "\"weekly\""
    );
    let back: RotationType = serde_json::from_str("\"custom\"").unwrap();
    assert_eq!(back, RotationType::Custom);
}

#[test]
fn escalation_step_type_serde_variants() {
    let tid = Uuid::new_v4();
    let wait = EscalationStepType::Wait { minutes: 10 };
    let user = EscalationStepType::NotifyUser {
        username: "alice".into(),
    };
    let team = EscalationStepType::NotifyTeam { team_id: tid };
    let repeat = EscalationStepType::RepeatFromStart;

    // Round-trip each variant via JSON.
    for v in [
        EscalationStepType::NotifyOnCall,
        wait.clone(),
        user.clone(),
        team.clone(),
        repeat.clone(),
    ] {
        let json = serde_json::to_string(&v).unwrap();
        let back: EscalationStepType = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}

#[test]
fn alert_full_round_trip_preserves_all_fields() {
    let mut labels = HashMap::new();
    labels.insert("env".into(), "prod".into());
    let mut alert = firing_alert("fp-x", labels);
    alert.state = AlertState::Acknowledged;
    alert.ack_at = Some(Utc::now());
    alert.ack_by = Some("alice".into());
    alert.current_escalation_step = 3;

    let json = serde_json::to_string(&alert).unwrap();
    let back: Alert = serde_json::from_str(&json).unwrap();
    assert_eq!(alert, back);
}
