// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cron-based scheduling for chaos experiments.
//!
//! Maps to Chaos Mesh's `Schedule` CRD which allows experiments to run
//! automatically on a cron schedule.
//!
//! Implements a pure-Rust 5-field cron parser and evaluation engine.
//! No external cron library required.

use chrono::{DateTime, Datelike, Timelike, Utc};

use crate::models::ExperimentSchedule;

/// A parsed cron field value.
#[derive(Debug, Clone, PartialEq)]
pub enum CronField {
    /// `*` — matches any value.
    Wildcard,
    /// Exact value, e.g. `5`.
    Exact(u32),
    /// Step pattern, e.g. `*/5` — matches multiples of step.
    Step(u32),
    /// Range, e.g. `10-20` — matches inclusive.
    Range(u32, u32),
    /// List, e.g. `1,3,5`.
    List(Vec<u32>),
}

/// A parsed 5-field cron expression.
#[derive(Debug, Clone)]
pub struct ParsedCron {
    pub minute: CronField,
    pub hour: CronField,
    pub day_of_month: CronField,
    pub month: CronField,
    pub day_of_week: CronField,
}

/// Parse a single cron field string into a `CronField`.
fn parse_field(s: &str, _min: u32, _max: u32) -> Result<CronField, String> {
    if s == "*" {
        return Ok(CronField::Wildcard);
    }
    // */step
    if let Some(step_str) = s.strip_prefix("*/") {
        let step: u32 = step_str
            .parse()
            .map_err(|_| format!("invalid step in '{}'", s))?;
        if step == 0 {
            return Err(format!("step cannot be 0 in '{}'", s));
        }
        return Ok(CronField::Step(step));
    }
    // range: lo-hi
    if s.contains('-') && !s.contains(',') {
        let parts: Vec<&str> = s.splitn(2, '-').collect();
        let lo: u32 = parts[0]
            .parse()
            .map_err(|_| format!("invalid range lo in '{}'", s))?;
        let hi: u32 = parts[1]
            .parse()
            .map_err(|_| format!("invalid range hi in '{}'", s))?;
        if lo > hi {
            return Err(format!("range lo > hi in '{}'", s));
        }
        return Ok(CronField::Range(lo, hi));
    }
    // list: a,b,c
    if s.contains(',') {
        let mut vals = Vec::new();
        for part in s.split(',') {
            let v: u32 = part
                .parse()
                .map_err(|_| format!("invalid list value in '{}'", s))?;
            vals.push(v);
        }
        return Ok(CronField::List(vals));
    }
    // exact
    let v: u32 = s
        .parse()
        .map_err(|_| format!("invalid value in cron field '{}'", s))?;
    Ok(CronField::Exact(v))
}

/// Parse a 5-field cron expression string.
pub fn parse_cron(expr: &str) -> Result<ParsedCron, String> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(format!(
            "cron expression must have 5 fields, got {} in '{}'",
            parts.len(),
            expr
        ));
    }
    Ok(ParsedCron {
        minute: parse_field(parts[0], 0, 59)?,
        hour: parse_field(parts[1], 0, 23)?,
        day_of_month: parse_field(parts[2], 1, 31)?,
        month: parse_field(parts[3], 1, 12)?,
        day_of_week: parse_field(parts[4], 0, 7)?,
    })
}

/// Check if a `CronField` matches a given value.
pub fn cron_field_matches(field: &CronField, value: u32) -> bool {
    match field {
        CronField::Wildcard => true,
        CronField::Exact(v) => *v == value,
        CronField::Step(step) => value % step == 0,
        CronField::Range(lo, hi) => value >= *lo && value <= *hi,
        CronField::List(vals) => vals.contains(&value),
    }
}

/// Validate a cron expression string, checking field count and value ranges.
pub fn validate_cron_expression(expr: &str) -> Result<ParsedCron, String> {
    if expr.is_empty() {
        return Err("cron expression must not be empty".to_string());
    }
    let parsed = parse_cron(expr)?;

    // Range validation for each field
    validate_field_range(&parsed.minute, 0, 59, "minute")?;
    validate_field_range(&parsed.hour, 0, 23, "hour")?;
    validate_field_range(&parsed.day_of_month, 1, 31, "day_of_month")?;
    validate_field_range(&parsed.month, 1, 12, "month")?;
    // day_of_week: 0-7 (0 and 7 both mean Sunday)
    validate_field_range(&parsed.day_of_week, 0, 7, "day_of_week")?;

    Ok(parsed)
}

fn validate_field_range(field: &CronField, min: u32, max: u32, name: &str) -> Result<(), String> {
    let in_range = |v: u32| v >= min && v <= max;
    match field {
        CronField::Wildcard => {}
        CronField::Exact(v) => {
            if !in_range(*v) {
                return Err(format!(
                    "{} value {} out of range [{}, {}]",
                    name, v, min, max
                ));
            }
        }
        CronField::Step(step) => {
            if *step == 0 || *step > max {
                return Err(format!("{} step {} invalid", name, step));
            }
        }
        CronField::Range(lo, hi) => {
            if !in_range(*lo) || !in_range(*hi) {
                return Err(format!(
                    "{} range {}-{} out of range [{}, {}]",
                    name, lo, hi, min, max
                ));
            }
        }
        CronField::List(vals) => {
            for v in vals {
                if !in_range(*v) {
                    return Err(format!(
                        "{} list value {} out of range [{}, {}]",
                        name, v, min, max
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Returns `true` if the cron expression fires at the given `datetime`.
/// The cron fires at exact minute granularity.
pub fn is_cron_due(expr: &str, at: &DateTime<Utc>) -> Result<bool, String> {
    let parsed = validate_cron_expression(expr)?;
    let minute = at.minute();
    let hour = at.hour();
    let dom = at.day();
    let month = at.month();
    // chrono weekday: Mon=1..Sun=7; cron: 0=Sun..6=Sat, 7=Sun
    let dow = match at.weekday() {
        chrono::Weekday::Sun => 0u32,
        chrono::Weekday::Mon => 1,
        chrono::Weekday::Tue => 2,
        chrono::Weekday::Wed => 3,
        chrono::Weekday::Thu => 4,
        chrono::Weekday::Fri => 5,
        chrono::Weekday::Sat => 6,
    };

    Ok(cron_field_matches(&parsed.minute, minute)
        && cron_field_matches(&parsed.hour, hour)
        && cron_field_matches(&parsed.day_of_month, dom)
        && cron_field_matches(&parsed.month, month)
        && cron_field_matches(&parsed.day_of_week, dow))
}

/// Compute the next time the cron expression will fire after `after`.
/// Scans forward minute-by-minute for up to 366 days (365d * 24h * 60m ≈ 527k minutes).
pub fn next_cron_run(expr: &str, after: &DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    let parsed = validate_cron_expression(expr)?;
    // Start from the next minute
    let mut candidate = after
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(*after)
        + chrono::Duration::minutes(1);

    let limit = *after + chrono::Duration::days(366);
    while candidate <= limit {
        let minute = candidate.minute();
        let hour = candidate.hour();
        let dom = candidate.day();
        let month = candidate.month();
        let dow = match candidate.weekday() {
            chrono::Weekday::Sun => 0u32,
            chrono::Weekday::Mon => 1,
            chrono::Weekday::Tue => 2,
            chrono::Weekday::Wed => 3,
            chrono::Weekday::Thu => 4,
            chrono::Weekday::Fri => 5,
            chrono::Weekday::Sat => 6,
        };
        if cron_field_matches(&parsed.minute, minute)
            && cron_field_matches(&parsed.hour, hour)
            && cron_field_matches(&parsed.day_of_month, dom)
            && cron_field_matches(&parsed.month, month)
            && cron_field_matches(&parsed.day_of_week, dow)
        {
            return Ok(candidate);
        }
        candidate = candidate + chrono::Duration::minutes(1);
    }
    Err(format!(
        "no matching time found within 366 days for cron '{}'",
        expr
    ))
}

/// Decision for whether a scheduled experiment should run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduledRunDecision {
    /// The schedule is due and should run.
    Run,
    /// The schedule is disabled or not yet due.
    Skip,
    /// The schedule has reached its max_runs limit.
    Exhausted,
}

/// Determine whether a scheduled experiment should run at `now`.
pub fn should_run(sched: &ExperimentSchedule, now: &DateTime<Utc>) -> ScheduledRunDecision {
    if !sched.enabled {
        return ScheduledRunDecision::Skip;
    }
    if let Some(max) = sched.max_runs {
        if sched.run_count >= max {
            return ScheduledRunDecision::Exhausted;
        }
    }
    // If already ran this minute, skip
    if let Some(last_run) = sched.last_run {
        let same_minute = last_run.date_naive() == now.date_naive()
            && last_run.hour() == now.hour()
            && last_run.minute() == now.minute();
        if same_minute {
            return ScheduledRunDecision::Skip;
        }
    }
    // Check if the cron fires at `now`
    match is_cron_due(&sched.cron_expression, now) {
        Ok(true) => ScheduledRunDecision::Run,
        _ => ScheduledRunDecision::Skip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;
    use crate::models::ExperimentSchedule;

    #[test]
    fn test_wildcard_matches_all() {
        let f = CronField::Wildcard;
        for v in 0..=59 {
            assert!(cron_field_matches(&f, v));
        }
    }

    #[test]
    fn test_exact_field() {
        let f = CronField::Exact(15);
        assert!(cron_field_matches(&f, 15));
        assert!(!cron_field_matches(&f, 14));
    }

    #[test]
    fn test_step_field() {
        let f = CronField::Step(5);
        assert!(cron_field_matches(&f, 0));
        assert!(cron_field_matches(&f, 5));
        assert!(cron_field_matches(&f, 10));
        assert!(!cron_field_matches(&f, 3));
    }

    #[test]
    fn test_range_field() {
        let f = CronField::Range(10, 20);
        assert!(cron_field_matches(&f, 10));
        assert!(cron_field_matches(&f, 15));
        assert!(cron_field_matches(&f, 20));
        assert!(!cron_field_matches(&f, 9));
        assert!(!cron_field_matches(&f, 21));
    }

    #[test]
    fn test_list_field() {
        let f = CronField::List(vec![1, 3, 5]);
        assert!(cron_field_matches(&f, 1));
        assert!(cron_field_matches(&f, 5));
        assert!(!cron_field_matches(&f, 2));
    }

    #[test]
    fn test_validate_valid_expressions() {
        assert!(validate_cron_expression("* * * * *").is_ok());
        assert!(validate_cron_expression("0 2 * * 1").is_ok());
        assert!(validate_cron_expression("*/5 * * * *").is_ok());
    }

    #[test]
    fn test_validate_bad_expressions() {
        assert!(validate_cron_expression("").is_err());
        assert!(validate_cron_expression("* * * *").is_err()); // 4 fields
        assert!(validate_cron_expression("60 * * * *").is_err()); // minute 60 out of range
        assert!(validate_cron_expression("* 25 * * *").is_err()); // hour 25
    }

    #[test]
    fn test_is_cron_due_matching() {
        // 2026-06-01 Monday 02:00 UTC
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 2, 0, 0).unwrap();
        assert!(is_cron_due("0 2 * * 1", &t).unwrap());
    }

    #[test]
    fn test_is_cron_due_not_matching() {
        let t = Utc.with_ymd_and_hms(2026, 6, 1, 3, 0, 0).unwrap();
        assert!(!is_cron_due("0 2 * * 1", &t).unwrap());
    }

    #[test]
    fn test_next_cron_run_returns_future() {
        let now = Utc::now();
        let next = next_cron_run("0 * * * *", &now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn test_should_run_when_due() {
        let sched = ExperimentSchedule {
            id: Uuid::new_v4(),
            experiment_id: Uuid::new_v4(),
            cron_expression: "* * * * *".into(),
            enabled: true,
            last_run: None,
            next_run: None,
            max_runs: None,
            run_count: 0,
        };
        assert_eq!(should_run(&sched, &Utc::now()), ScheduledRunDecision::Run);
    }

    #[test]
    fn test_should_skip_when_disabled() {
        let sched = ExperimentSchedule {
            id: Uuid::new_v4(),
            experiment_id: Uuid::new_v4(),
            cron_expression: "* * * * *".into(),
            enabled: false,
            last_run: None,
            next_run: None,
            max_runs: None,
            run_count: 0,
        };
        assert_eq!(should_run(&sched, &Utc::now()), ScheduledRunDecision::Skip);
    }

    #[test]
    fn test_should_be_exhausted() {
        let sched = ExperimentSchedule {
            id: Uuid::new_v4(),
            experiment_id: Uuid::new_v4(),
            cron_expression: "* * * * *".into(),
            enabled: true,
            last_run: None,
            next_run: None,
            max_runs: Some(3),
            run_count: 3,
        };
        assert_eq!(should_run(&sched, &Utc::now()), ScheduledRunDecision::Exhausted);
    }
}
