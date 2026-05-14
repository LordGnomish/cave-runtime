// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Schedule validation and cron utilities.

use crate::models::Schedule;

/// Validate a cron expression (basic: check field count = 5).
pub fn validate_cron(expr: &str) -> bool {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    parts.len() == 5
}

/// Compute a human-readable description of a cron expression.
pub fn describe_cron(expr: &str) -> String {
    match expr {
        "0 * * * *" => "every hour".into(),
        "0 0 * * *" => "daily at midnight".into(),
        "0 0 * * 0" => "weekly on Sunday".into(),
        "0 0 1 * *" => "monthly on the 1st".into(),
        _ => format!("cron: {expr}"),
    }
}

/// Return IDs of all schedules that are not paused (eligible to fire).
pub fn due_schedules(schedules: &[&Schedule]) -> Vec<uuid::Uuid> {
    schedules
        .iter()
        .filter(|s| !s.paused)
        .map(|s| s.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BackupSpec, Schedule};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_schedule(paused: bool) -> Schedule {
        Schedule {
            id: Uuid::new_v4(),
            name: "test".into(),
            cron_expression: "0 0 * * *".into(),
            template: BackupSpec::default(),
            paused,
            last_backup_at: None,
            last_backup_phase: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn validate_cron_accepts_five_fields() {
        assert!(validate_cron("0 * * * *"));
        assert!(validate_cron("*/15 * * * *"));
        assert!(validate_cron("0 0 * * 0"));
        assert!(validate_cron("30 2 1 * *"));
    }

    #[test]
    fn validate_cron_rejects_wrong_field_count() {
        assert!(!validate_cron("* * * *"));         // 4 fields
        assert!(!validate_cron("* * * * * *"));     // 6 fields
        assert!(!validate_cron(""));                // 0 fields
        assert!(!validate_cron("0 0 * * * extra")); // 6 fields
    }

    #[test]
    fn describe_cron_known_expressions() {
        assert_eq!(describe_cron("0 * * * *"), "every hour");
        assert_eq!(describe_cron("0 0 * * *"), "daily at midnight");
        assert_eq!(describe_cron("0 0 * * 0"), "weekly on Sunday");
        assert_eq!(describe_cron("0 0 1 * *"), "monthly on the 1st");
    }

    #[test]
    fn describe_cron_unknown_falls_through() {
        let desc = describe_cron("15 3 * * 2");
        assert!(desc.contains("15 3 * * 2"));
    }

    #[test]
    fn due_schedules_excludes_paused() {
        let active = make_schedule(false);
        let paused = make_schedule(true);
        let active_id = active.id;

        let refs: Vec<&Schedule> = vec![&active, &paused];
        let due = due_schedules(&refs);
        assert_eq!(due, vec![active_id]);
    }

    #[test]
    fn due_schedules_empty_when_all_paused() {
        let s1 = make_schedule(true);
        let s2 = make_schedule(true);
        let refs: Vec<&Schedule> = vec![&s1, &s2];
        assert!(due_schedules(&refs).is_empty());
    }

    #[test]
    fn due_schedules_all_when_none_paused() {
        let s1 = make_schedule(false);
        let s2 = make_schedule(false);
        let refs: Vec<&Schedule> = vec![&s1, &s2];
        assert_eq!(due_schedules(&refs).len(), 2);
    }
}
