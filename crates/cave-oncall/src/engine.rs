use crate::models::{
    Alert, AlertState, EscalationPolicy, EscalationStep, OnCallAssignment, Rotation, Schedule,
    ShiftOverride, Silence,
};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum OnCallError {
    #[error("Invalid rotation: {0}")]
    InvalidRotation(String),
    #[error("Team not found")]
    TeamNotFound,
    #[error("Schedule not found")]
    ScheduleNotFound,
    #[error("User not found")]
    UserNotFound,
    #[error("Alert not found")]
    AlertNotFound,
    #[error("Invalid time range")]
    InvalidTimeRange,
    #[error("Alert already acknowledged")]
    AlreadyAcknowledged,
}

// ---------------------------------------------------------------------------
// Core business logic functions
// ---------------------------------------------------------------------------

/// Determine who is on-call for a schedule at a specific moment.
/// Returns the current on-call assignment if one exists, considering:
/// - Active overrides (highest priority)
/// - Active rotations (if no override)
/// - Start and end times
pub fn current_oncall(
    schedule: &Schedule,
    rotations: &[Rotation],
    overrides: &[ShiftOverride],
    at: DateTime<Utc>,
) -> Option<OnCallAssignment> {
    // First, check for overrides at this moment
    for override_shift in overrides {
        if override_shift.schedule_id == schedule.id
            && override_shift.start <= at
            && at < override_shift.end
        {
            return Some(OnCallAssignment {
                team_id: schedule.team_id,
                user: override_shift.user.clone(),
                start: override_shift.start,
                end: override_shift.end,
                schedule_id: schedule.id,
                rotation_id: None,
                is_override: true,
            });
        }
    }

    // Check active rotations
    for rotation in rotations {
        if !rotation.active || rotation.schedule_id != schedule.id {
            continue;
        }

        if let Some(assignment) = rotation_assignment_at(rotation, at) {
            return Some(OnCallAssignment {
                team_id: schedule.team_id,
                user: assignment.user,
                start: assignment.start,
                end: assignment.end,
                schedule_id: schedule.id,
                rotation_id: Some(rotation.id),
                is_override: false,
            });
        }
    }

    None
}

/// Calculate the on-call assignment for a rotation at a given moment.
fn rotation_assignment_at(rotation: &Rotation, at: DateTime<Utc>) -> Option<OnCallAssignment> {
    if rotation.users.is_empty() || at < rotation.start_date {
        return None;
    }

    let shift_duration = Duration::hours(rotation.shift_duration_hours as i64);
    let elapsed = at.signed_duration_since(rotation.start_date);

    // Calculate which user is on-call based on rotation type
    let user_index = match rotation.rotation_type {
        crate::models::RotationType::Daily => {
            // Each user gets one day
            let days_elapsed = elapsed.num_days();
            (days_elapsed as usize) % rotation.users.len()
        }
        crate::models::RotationType::Weekly => {
            // Each user gets one week
            let weeks_elapsed = elapsed.num_weeks();
            (weeks_elapsed as usize) % rotation.users.len()
        }
        crate::models::RotationType::Custom => {
            // Custom: use shift duration to calculate cycles.
            let shift_secs = shift_duration.num_seconds().max(1);
            let cycles = elapsed.num_seconds() / shift_secs;
            (cycles as usize) % rotation.users.len()
        }
    };

    if user_index >= rotation.users.len() {
        return None;
    }

    let user = rotation.users[user_index].clone();
    let shift_start = rotation.start_date
        + Duration::days((user_index as i64) * rotation.shift_duration_hours as i64 / 24);
    let shift_end = shift_start + shift_duration;

    Some(OnCallAssignment {
        team_id: Uuid::nil(), // Will be filled in by caller
        user,
        start: shift_start,
        end: shift_end,
        schedule_id: rotation.schedule_id,
        rotation_id: Some(rotation.id),
        is_override: false,
    })
}

use uuid::Uuid;

/// Get upcoming shifts for a schedule within the next N days.
pub fn upcoming_shifts(
    schedule: &Schedule,
    rotations: &[Rotation],
    horizon_days: u32,
) -> Vec<OnCallAssignment> {
    let mut shifts = Vec::new();
    let now = Utc::now();
    let horizon = now + Duration::days(horizon_days as i64);

    for rotation in rotations {
        if !rotation.active || rotation.schedule_id != schedule.id {
            continue;
        }

        let shift_duration = Duration::hours(rotation.shift_duration_hours as i64);
        let mut current_time = rotation.start_date;

        while current_time < horizon {
            let user_index = if rotation.users.is_empty() {
                break;
            } else {
                let elapsed = current_time.signed_duration_since(rotation.start_date);
                let cycles = match rotation.rotation_type {
                    crate::models::RotationType::Daily => elapsed.num_days(),
                    crate::models::RotationType::Weekly => elapsed.num_weeks(),
                    crate::models::RotationType::Custom => {
                        let shift_secs = shift_duration.num_seconds().max(1);
                        elapsed.num_seconds() / shift_secs
                    }
                };
                (cycles as usize) % rotation.users.len()
            };

            shifts.push(OnCallAssignment {
                team_id: schedule.team_id,
                user: rotation.users[user_index].clone(),
                start: current_time,
                end: current_time + shift_duration,
                schedule_id: schedule.id,
                rotation_id: Some(rotation.id),
                is_override: false,
            });

            current_time = current_time + shift_duration;
        }
    }

    shifts.sort_by_key(|s| s.start);
    shifts
}

/// Check if an alert with the given fingerprint already exists in the store.
/// Returns the existing alert's ID if found, None otherwise (for deduplication).
pub fn dedupe_fingerprint(fingerprint: &str, existing: &[Alert]) -> Option<Uuid> {
    existing
        .iter()
        .find(|a| a.fingerprint == fingerprint && a.state != AlertState::Resolved)
        .map(|a| a.id)
}

/// Evaluate whether an alert should be silenced based on active silences.
/// Returns true if the alert matches any active silence at the given time.
pub fn evaluate_silences(alert: &Alert, silences: &[Silence], at: DateTime<Utc>) -> bool {
    silences.iter().any(|s| {
        // Silence must be active at this time
        if !(s.start <= at && at < s.end) {
            return false;
        }
        // All matchers must match the alert's labels
        s.matcher.iter().all(|(key, value)| {
            alert.labels.get(key).map_or(false, |v| v == value)
        })
    })
}

/// Determine the next escalation step to execute for an alert.
/// Returns the step that should be executed now, or None if all steps are exhausted.
pub fn next_escalation_step<'a>(
    alert: &Alert,
    policy: &'a EscalationPolicy,
    elapsed_seconds: u32,
) -> Option<&'a EscalationStep> {
    policy.steps.iter().find(|step| {
        // The step's timeout must have been reached
        let cumulative_timeout: u32 = policy
            .steps
            .iter()
            .take_while(|s| s.order < step.order)
            .map(|s| s.timeout_seconds)
            .sum();

        elapsed_seconds >= cumulative_timeout
            && (alert.current_escalation_step <= step.order)
    })
}

/// Validate a rotation configuration for basic sanity.
pub fn validate_rotation(rot: &Rotation) -> Result<(), OnCallError> {
    if rot.users.is_empty() {
        return Err(OnCallError::InvalidRotation(
            "rotation must have at least one user".to_string(),
        ));
    }

    if rot.handoff_hour > 23 {
        return Err(OnCallError::InvalidRotation(
            "handoff_hour must be 0-23".to_string(),
        ));
    }

    if rot.handoff_minute > 59 {
        return Err(OnCallError::InvalidRotation(
            "handoff_minute must be 0-59".to_string(),
        ));
    }

    if rot.shift_duration_hours == 0 {
        return Err(OnCallError::InvalidRotation(
            "shift_duration_hours must be greater than 0".to_string(),
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RotationType, Schedule, ScheduleType};
    use chrono::TimeZone;

    fn sample_schedule() -> Schedule {
        Schedule {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            name: "Primary".to_string(),
            description: None,
            timezone: "UTC".to_string(),
            schedule_type: ScheduleType::Rotation,
            created_at: Utc::now(),
        }
    }

    fn sample_rotation() -> Rotation {
        let now = Utc::now();
        Rotation {
            id: Uuid::new_v4(),
            schedule_id: Uuid::new_v4(),
            name: "Weekly".to_string(),
            users: vec!["alice".to_string(), "bob".to_string()],
            start_date: now,
            rotation_type: RotationType::Weekly,
            handoff_hour: 9,
            handoff_minute: 0,
            shift_duration_hours: 168, // 7 days
            active: true,
        }
    }

    #[test]
    fn test_current_oncall_with_override() {
        let schedule = sample_schedule();
        let rotation = sample_rotation();
        let now = Utc::now();
        let override_shift = ShiftOverride {
            id: Uuid::new_v4(),
            schedule_id: schedule.id,
            user: "charlie".to_string(),
            start: now - Duration::hours(1),
            end: now + Duration::hours(1),
            reason: Some("manual override".to_string()),
        };

        let result = current_oncall(&schedule, &[], &[override_shift], now);
        assert!(result.is_some());
        let assignment = result.unwrap();
        assert_eq!(assignment.user, "charlie");
        assert!(assignment.is_override);
    }

    #[test]
    fn test_current_oncall_weekly_rotation_boundary() {
        let mut schedule = sample_schedule();
        let mut rotation = sample_rotation();
        schedule.id = rotation.schedule_id;

        // Start at week 0, check at week 0 and week 1 boundary
        let start = Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap(); // Monday
        rotation.start_date = start;

        // At start: alice should be on-call
        let result_start = current_oncall(&schedule, &[rotation.clone()], &[], start);
        assert!(result_start.is_some());
        assert_eq!(result_start.unwrap().user, "alice");

        // After 1 week: bob should be on-call
        let week_later = start + Duration::weeks(1);
        let result_later = current_oncall(&schedule, &[rotation], &[], week_later);
        assert!(result_later.is_some());
        assert_eq!(result_later.unwrap().user, "bob");
    }

    #[test]
    fn test_upcoming_shifts() {
        let mut schedule = sample_schedule();
        let mut rotation = sample_rotation();
        schedule.id = rotation.schedule_id;

        let shifts = upcoming_shifts(&schedule, &[rotation], 14);
        assert!(!shifts.is_empty());
        assert!(shifts.iter().all(|s| s.schedule_id == schedule.id));
    }

    #[test]
    fn test_dedupe_fingerprint() {
        let alert = Alert {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            title: "Test".to_string(),
            severity: crate::models::Severity::High,
            source: "prometheus".to_string(),
            fingerprint: "abc123".to_string(),
            state: AlertState::Firing,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            created_at: Utc::now(),
            ack_at: None,
            ack_by: None,
            resolved_at: None,
            escalation_policy_id: None,
            current_escalation_step: 0,
        };

        let existing = vec![alert.clone()];
        let found = dedupe_fingerprint("abc123", &existing);
        assert_eq!(found, Some(alert.id));

        let not_found = dedupe_fingerprint("xyz789", &existing);
        assert_eq!(not_found, None);
    }

    #[test]
    fn test_evaluate_silences_match() {
        let mut labels = HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());

        let alert = Alert {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            title: "Test".to_string(),
            severity: crate::models::Severity::High,
            source: "prometheus".to_string(),
            fingerprint: "abc123".to_string(),
            state: AlertState::Firing,
            labels,
            annotations: HashMap::new(),
            created_at: Utc::now(),
            ack_at: None,
            ack_by: None,
            resolved_at: None,
            escalation_policy_id: None,
            current_escalation_step: 0,
        };

        let now = Utc::now();
        let mut matcher = HashMap::new();
        matcher.insert("env".to_string(), "prod".to_string());

        let silence = Silence {
            id: Uuid::new_v4(),
            team_id: alert.team_id,
            matcher,
            start: now - Duration::hours(1),
            end: now + Duration::hours(1),
            created_by: "alice".to_string(),
            reason: None,
        };

        assert!(evaluate_silences(&alert, &[silence], now));
    }

    #[test]
    fn test_evaluate_silences_no_match() {
        let alert = Alert {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            title: "Test".to_string(),
            severity: crate::models::Severity::High,
            source: "prometheus".to_string(),
            fingerprint: "abc123".to_string(),
            state: AlertState::Firing,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            created_at: Utc::now(),
            ack_at: None,
            ack_by: None,
            resolved_at: None,
            escalation_policy_id: None,
            current_escalation_step: 0,
        };

        let now = Utc::now();
        let mut matcher = HashMap::new();
        matcher.insert("env".to_string(), "prod".to_string());

        let silence = Silence {
            id: Uuid::new_v4(),
            team_id: alert.team_id,
            matcher,
            start: now - Duration::hours(1),
            end: now + Duration::hours(1),
            created_by: "alice".to_string(),
            reason: None,
        };

        assert!(!evaluate_silences(&alert, &[silence], now));
    }

    #[test]
    fn test_next_escalation_step_progression() {
        let policy = EscalationPolicy {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            name: "Escalate".to_string(),
            steps: vec![
                EscalationStep {
                    order: 0,
                    step_type: crate::models::EscalationStepType::NotifyOnCall,
                    timeout_seconds: 300,
                },
                EscalationStep {
                    order: 1,
                    step_type: crate::models::EscalationStepType::NotifyOnCall,
                    timeout_seconds: 300,
                },
            ],
            created_at: Utc::now(),
        };

        let mut alert = Alert {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            title: "Test".to_string(),
            severity: crate::models::Severity::High,
            source: "prometheus".to_string(),
            fingerprint: "abc123".to_string(),
            state: AlertState::Firing,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            created_at: Utc::now(),
            ack_at: None,
            ack_by: None,
            resolved_at: None,
            escalation_policy_id: Some(policy.id),
            current_escalation_step: 0,
        };

        // At 100 seconds, step 0 should execute
        let step = next_escalation_step(&alert, &policy, 100);
        assert!(step.is_some());
        assert_eq!(step.unwrap().order, 0);

        // At 400 seconds (after first timeout), step 1 should execute
        let step = next_escalation_step(&alert, &policy, 400);
        assert!(step.is_some());
        assert_eq!(step.unwrap().order, 1);

        // After marking step 1 as current, no new steps
        alert.current_escalation_step = 1;
        let step = next_escalation_step(&alert, &policy, 400);
        assert!(step.is_some());
        assert_eq!(step.unwrap().order, 1);
    }

    #[test]
    fn test_validate_rotation_empty_users() {
        let mut rotation = sample_rotation();
        rotation.users = Vec::new();

        let result = validate_rotation(&rotation);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_rotation_invalid_handoff_hour() {
        let mut rotation = sample_rotation();
        rotation.handoff_hour = 25;

        let result = validate_rotation(&rotation);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_rotation_zero_duration() {
        let mut rotation = sample_rotation();
        rotation.shift_duration_hours = 0;

        let result = validate_rotation(&rotation);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_rotation_valid() {
        let rotation = sample_rotation();
        let result = validate_rotation(&rotation);
        assert!(result.is_ok());
    }
}
