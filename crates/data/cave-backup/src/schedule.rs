// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Schedule validation and cron utilities.

use crate::models::Schedule;
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};

/// Validate a cron expression (basic: check field count = 5).
pub fn validate_cron(expr: &str) -> bool {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    parts.len() == 5
}

/// A parsed standard 5-field cron expression: minute hour day-of-month month
/// day-of-week. Each field is either a wildcard or an explicit set of allowed
/// values, plus a step (e.g. `*/15`). This is the focused subset needed by the
/// scheduler — ports the matching semantics of Velero's robfig/cron usage.
#[derive(Debug, Clone)]
struct CronSchedule {
    minute: Vec<u32>,
    hour: Vec<u32>,
    dom: Vec<u32>,
    month: Vec<u32>,
    dow: Vec<u32>,
    dom_restricted: bool,
    dow_restricted: bool,
}

/// Parse a single cron field into the set of values it matches within
/// `[min, max]`. Supports `*`, `*/step`, explicit values, and comma lists.
fn parse_field(field: &str, min: u32, max: u32) -> Option<(Vec<u32>, bool)> {
    let mut values = Vec::new();
    let restricted = field != "*" && !field.starts_with("*/");

    for part in field.split(',') {
        if let Some(step_str) = part.strip_prefix("*/") {
            let step: u32 = step_str.parse().ok()?;
            if step == 0 {
                return None;
            }
            let mut v = min;
            while v <= max {
                values.push(v);
                v += step;
            }
        } else if part == "*" {
            for v in min..=max {
                values.push(v);
            }
        } else if let Some((lo, hi)) = part.split_once('-') {
            let lo: u32 = lo.parse().ok()?;
            let hi: u32 = hi.parse().ok()?;
            if lo > hi || lo < min || hi > max {
                return None;
            }
            for v in lo..=hi {
                values.push(v);
            }
        } else {
            let v: u32 = part.parse().ok()?;
            if v < min || v > max {
                return None;
            }
            values.push(v);
        }
    }

    values.sort_unstable();
    values.dedup();
    if values.is_empty() {
        return None;
    }
    Some((values, restricted))
}

impl CronSchedule {
    fn parse(expr: &str) -> Option<Self> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return None;
        }
        let (minute, _) = parse_field(parts[0], 0, 59)?;
        let (hour, _) = parse_field(parts[1], 0, 23)?;
        let (dom, dom_restricted) = parse_field(parts[2], 1, 31)?;
        let (month, _) = parse_field(parts[3], 1, 12)?;
        let (dow, dow_restricted) = parse_field(parts[4], 0, 6)?;
        Some(CronSchedule {
            minute,
            hour,
            dom,
            month,
            dow,
            dom_restricted,
            dow_restricted,
        })
    }

    /// Whether the given instant (minute resolution) matches all fields.
    /// Day-of-month and day-of-week combine with OR when both are restricted
    /// (matching standard cron / Velero's robfig semantics).
    fn matches(&self, dt: &DateTime<Utc>) -> bool {
        if !self.minute.contains(&dt.minute())
            || !self.hour.contains(&dt.hour())
            || !self.month.contains(&dt.month())
        {
            return false;
        }
        let dom_match = self.dom.contains(&dt.day());
        // chrono weekday: Mon=0..Sun=6 via num_days_from_monday; cron uses
        // Sun=0..Sat=6. Convert.
        let cron_dow = dt.weekday().num_days_from_sunday();
        let dow_match = self.dow.contains(&cron_dow);

        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => dom_match || dow_match,
            (true, false) => dom_match,
            (false, true) => dow_match,
            (false, false) => true,
        }
    }
}

/// Compute the next time the cron expression fires strictly after `after`.
///
/// Ports Velero `pkg/controller/schedule_controller.go::getNextRunTime`,
/// which delegates to a cron parser's `Next(after)`. We scan forward at
/// minute resolution (truncating sub-minute precision) up to a bounded
/// horizon. Returns `None` for an unparseable expression or if no match is
/// found within ~4 years.
pub fn next_run(cron_expr: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let schedule = CronSchedule::parse(cron_expr)?;

    // Start at the next whole minute strictly after `after` (cron fires at
    // minute boundaries; the run must be strictly later than `after`).
    let mut candidate = Utc
        .with_ymd_and_hms(
            after.year(),
            after.month(),
            after.day(),
            after.hour(),
            after.minute(),
            0,
        )
        .single()?
        + Duration::minutes(1);

    // Bound the search: 4 years of minutes is plenty for any 5-field cron.
    let max_iterations = 4 * 366 * 24 * 60;
    for _ in 0..max_iterations {
        if schedule.matches(&candidate) {
            return Some(candidate);
        }
        candidate += Duration::minutes(1);
    }
    None
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
        assert!(!validate_cron("* * * *")); // 4 fields
        assert!(!validate_cron("* * * * * *")); // 6 fields
        assert!(!validate_cron("")); // 0 fields
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
