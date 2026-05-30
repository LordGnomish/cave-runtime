// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD coverage fills for `cave-incidents::oncall` — the pure on-call rotation
//! and escalation-snapshot slice ported from grafana/oncall v1.10.0.
//!
//! These exercise `OnCallEngine::{current_oncall, upcoming_shifts, escalate,
//! page_oncall}`, which port upstream "who is on-call now" (iCal schedule shift),
//! upcoming-shift enumeration, escalation target fan-out, and the "page current
//! on-call" backend path. All expected values are derived directly from the
//! rotation arithmetic in `src/oncall.rs`:
//!   rotation_index = (current_index + periods_elapsed) % users.len()
//! where periods_elapsed = floor(elapsed_secs / (rotation_period_days * 86400)).

use cave_incidents::models::{
    EscalationPolicy, EscalationStep, EscalationTarget, Incident, IncidentSeverity, IncidentStatus,
    NotificationChannel, OnCallSchedule, OnCallUser, ResponderRole, RotationType, ScheduleLayer,
};
use cave_incidents::oncall::{default_escalation_policy, default_schedule, OnCallEngine};
use chrono::{Duration, Utc};
use uuid::Uuid;

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn make_user(name: &str) -> OnCallUser {
    OnCallUser {
        id: Uuid::new_v4(),
        name: name.to_string(),
        email: format!("{}@example.com", name.to_lowercase()),
        phone: None,
        notification_prefs: vec![NotificationChannel::Email],
    }
}

/// A weekly layer with the given users whose rotation started `weeks_ago` weeks
/// in the past and `current_index == 0`.
fn weekly_schedule(users: Vec<OnCallUser>, weeks_ago: i64) -> OnCallSchedule {
    let layer = ScheduleLayer {
        id: Uuid::new_v4(),
        name: "Primary".to_string(),
        rotation_type: RotationType::Weekly,
        rotation_period_days: 7,
        users,
        current_index: 0,
        starts_at: Utc::now() - Duration::weeks(weeks_ago),
    };
    OnCallSchedule {
        id: Uuid::new_v4(),
        name: "Test Schedule".to_string(),
        timezone: "UTC".to_string(),
        layers: vec![layer],
    }
}

fn make_incident() -> Incident {
    let now = Utc::now();
    Incident {
        id: Uuid::new_v4(),
        title: "Database down".to_string(),
        description: "Primary DB is unreachable".to_string(),
        severity: IncidentSeverity::P1,
        status: IncidentStatus::Open,
        created_at: now,
        updated_at: now,
        acknowledged_at: None,
        resolved_at: None,
        created_by: Uuid::new_v4(),
        assigned_to: None,
        timeline: vec![],
        tags: vec![],
        responders: vec![],
    }
}

// ── current_oncall ──────────────────────────────────────────────────────────────

#[test]
fn current_oncall_advances_by_elapsed_periods() {
    // 3 users, weekly rotation, started ~2 weeks ago, current_index = 0.
    // periods_elapsed = floor(elapsed / 7d). At slightly-over 2 weeks this is 2,
    // so rotation_index = (0 + 2) % 3 = 2 → the third user, "Carol".
    let users = vec![make_user("Alice"), make_user("Bob"), make_user("Carol")];
    let schedule = weekly_schedule(users, 2);

    // Probe a moment a few hours after the 2-week boundary to keep
    // periods_elapsed firmly at 2 (avoids any clock-edge flake at exactly 14d).
    let at = schedule.layers[0].starts_at + Duration::days(14) + Duration::hours(3);

    let engine = OnCallEngine::new();
    let user = engine
        .current_oncall(&schedule, at)
        .expect("non-empty layer yields a current on-call user");
    assert_eq!(user.name, "Carol");
}

#[test]
fn current_oncall_at_start_is_first_user() {
    // Zero periods elapsed → index (0 + 0) % 3 = 0 → first user "Alice".
    let users = vec![make_user("Alice"), make_user("Bob"), make_user("Carol")];
    let schedule = weekly_schedule(users, 0);
    let at = schedule.layers[0].starts_at + Duration::hours(1);

    let engine = OnCallEngine::new();
    let user = engine.current_oncall(&schedule, at).unwrap();
    assert_eq!(user.name, "Alice");
}

#[test]
fn current_oncall_empty_users_is_none() {
    let schedule = weekly_schedule(vec![], 1);
    let engine = OnCallEngine::new();
    assert!(engine.current_oncall(&schedule, Utc::now()).is_none());
}

#[test]
fn current_oncall_no_layers_is_none() {
    let schedule = OnCallSchedule {
        id: Uuid::new_v4(),
        name: "Empty".to_string(),
        timezone: "UTC".to_string(),
        layers: vec![],
    };
    let engine = OnCallEngine::new();
    assert!(engine.current_oncall(&schedule, Utc::now()).is_none());
}

// ── upcoming_shifts ─────────────────────────────────────────────────────────────

#[test]
fn upcoming_shifts_count_period_and_contiguity() {
    // 3 users, weekly (period_secs = 7 * 86_400).
    let users = vec![make_user("Alice"), make_user("Bob"), make_user("Carol")];
    let schedule = weekly_schedule(users, 1);
    let period_secs = 7 * 86_400;

    let engine = OnCallEngine::new();
    let shifts = engine.upcoming_shifts(&schedule, 3);

    // count == 3 returns exactly 3 shifts.
    assert_eq!(shifts.len(), 3);

    for (start, end, _name) in &shifts {
        // Each shift spans exactly one rotation period.
        assert_eq!((*end - *start).num_seconds(), period_secs);
    }

    // Shifts are contiguous: shift[i].end == shift[i+1].start.
    assert_eq!(shifts[0].1, shifts[1].0);
    assert_eq!(shifts[1].1, shifts[2].0);
}

#[test]
fn upcoming_shifts_user_names_cycle_in_rotation_order() {
    // The on-call user cycles through the layer's users in order across
    // consecutive periods: idx advances by exactly 1 each shift (mod len).
    let users = vec![make_user("Alice"), make_user("Bob"), make_user("Carol")];
    let schedule = weekly_schedule(users, 1);

    let engine = OnCallEngine::new();
    let shifts = engine.upcoming_shifts(&schedule, 4);
    let names: Vec<&str> = shifts.iter().map(|(_, _, n)| n.as_str()).collect();

    // Three distinct users → the 4-shift window wraps: X, next, next, X again.
    assert_eq!(names[0], names[3]);
    assert_ne!(names[0], names[1]);
    assert_ne!(names[1], names[2]);
    assert_ne!(names[0], names[2]);
    // All names are drawn from the configured roster.
    for n in &names {
        assert!(["Alice", "Bob", "Carol"].contains(n));
    }
}

#[test]
fn upcoming_shifts_empty_users_is_empty() {
    let schedule = weekly_schedule(vec![], 1);
    let engine = OnCallEngine::new();
    assert!(engine.upcoming_shifts(&schedule, 3).is_empty());
}

// ── escalate ────────────────────────────────────────────────────────────────────

#[test]
fn escalate_in_range_step_yields_one_action_per_target() {
    // default_escalation_policy() has 3 steps, each with exactly one Team target,
    // so each in-range step produces exactly one action string.
    let policy = default_escalation_policy();
    let incident = make_incident();
    let engine = OnCallEngine::new();

    for step in 0..policy.steps.len() {
        let actions = engine.escalate(&incident, &policy, step);
        assert_eq!(actions.len(), policy.steps[step].targets.len());
        assert_eq!(actions.len(), 1);
    }

    // Step 0 targets the "primary-oncall" team → "notify team" wording.
    let step0 = engine.escalate(&incident, &policy, 0);
    assert!(step0[0].contains("primary-oncall"));
    assert!(step0[0].contains("notify team"));
    assert!(step0[0].contains("Database down"));
}

#[test]
fn escalate_out_of_range_step_returns_single_not_exist_message() {
    let policy = default_escalation_policy();
    let incident = make_incident();
    let engine = OnCallEngine::new();

    // policy has 3 steps (indices 0..=2); index 3 is out of range.
    let actions = engine.escalate(&incident, &policy, 3);
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0],
        format!("Step 3 does not exist in policy '{}'", policy.name)
    );
}

#[test]
fn escalate_multi_target_step_emits_one_action_each() {
    // A custom step with three heterogeneous targets must produce three actions,
    // one per target, in declaration order.
    let user_id = Uuid::new_v4();
    let sched_id = Uuid::new_v4();
    let policy = EscalationPolicy {
        id: Uuid::new_v4(),
        name: "Multi".to_string(),
        steps: vec![EscalationStep {
            delay_minutes: 0,
            targets: vec![
                EscalationTarget::User(user_id),
                EscalationTarget::Schedule(sched_id),
                EscalationTarget::Team("sre".to_string()),
            ],
        }],
        repeat_count: 1,
    };
    let incident = make_incident();
    let engine = OnCallEngine::new();

    let actions = engine.escalate(&incident, &policy, 0);
    assert_eq!(actions.len(), 3);
    assert!(actions[0].contains(&user_id.to_string()));
    assert!(actions[0].contains("page user"));
    assert!(actions[1].contains(&sched_id.to_string()));
    assert!(actions[1].contains("on-call from schedule"));
    assert!(actions[2].contains("sre"));
    assert!(actions[2].contains("notify team"));
}

// ── page_oncall ─────────────────────────────────────────────────────────────────

#[test]
fn page_oncall_returns_current_oncall_as_unacked_responder() {
    // default_schedule(): 3 users, weekly, starts_at = now, current_index = 0.
    // current_oncall(now) → index (0 + 0) % 3 = 0 → "Alice On-Call".
    let schedule = default_schedule();
    let expected = &schedule.layers[0].users[0];
    let incident = make_incident();
    let engine = OnCallEngine::new();

    let responder = engine
        .page_oncall(&incident, &schedule)
        .expect("schedule with users pages the current on-call");

    assert_eq!(responder.user_id, expected.id);
    assert_eq!(responder.name, expected.name);
    assert_eq!(responder.email, expected.email);
    assert_eq!(responder.role, ResponderRole::Responder);
    // Freshly paged — not yet acknowledged.
    assert!(responder.acknowledged_at.is_none());
}

#[test]
fn page_oncall_empty_schedule_is_none() {
    let schedule = weekly_schedule(vec![], 0);
    let incident = make_incident();
    let engine = OnCallEngine::new();
    assert!(engine.page_oncall(&incident, &schedule).is_none());
}
