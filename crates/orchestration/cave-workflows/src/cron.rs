// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CronWorkflow — port of `argoproj/argo-workflows`
//! `pkg/apis/workflow/v1alpha1/cron_workflow_types.go` +
//! `workflow/cron` scheduling policy.
//!
//! A CronWorkflow wraps a [`WorkflowSpec`] with a cron `schedule`, a
//! [`ConcurrencyPolicy`] (Allow / Forbid / Replace), an optional
//! `starting_deadline_seconds` (skip a fire that is already too late) and
//! success/failure history limits. This module ports the cron expression
//! evaluator (`next` fire time) and the run/skip/replace decision the cron
//! controller makes each tick — the pure logic, not the K8s informer.

use crate::workflow_crd::WorkflowSpec;
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// How to treat a new scheduled run while previous runs are still active.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ConcurrencyPolicy {
    /// Run regardless of active workflows (cron default).
    #[default]
    Allow,
    /// Skip the new run if any prior run is still active.
    Forbid,
    /// Terminate active runs and start the new one.
    Replace,
}

/// CronWorkflow spec (`CronWorkflowSpec`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CronWorkflowSpec {
    #[serde(rename = "workflowSpec")]
    pub workflow_spec: WorkflowSpec,
    pub schedule: String,
    #[serde(default, rename = "concurrencyPolicy")]
    pub concurrency_policy: ConcurrencyPolicy,
    #[serde(default)]
    pub suspend: bool,
    #[serde(default, rename = "startingDeadlineSeconds")]
    pub starting_deadline_seconds: Option<i64>,
    #[serde(default, rename = "successfulJobsHistoryLimit")]
    pub successful_jobs_history_limit: Option<u32>,
    #[serde(default, rename = "failedJobsHistoryLimit")]
    pub failed_jobs_history_limit: Option<u32>,
    #[serde(default)]
    pub timezone: Option<String>,
}

/// CronWorkflow status (`CronWorkflowStatus`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CronWorkflowStatus {
    #[serde(default)]
    pub active: Vec<Uuid>,
    #[serde(default, rename = "lastScheduledTime")]
    pub last_scheduled_time: Option<DateTime<Utc>>,
}

/// Parse error for a cron expression.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CronError {
    #[error("cron schedule must have 5 fields, got {0}")]
    FieldCount(usize),
    #[error("cron field `{0}` out of range or malformed")]
    BadField(String),
}

/// A parsed 5-field cron schedule (minute hour day-of-month month day-of-week).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CronSchedule {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    doms: BTreeSet<u32>,
    months: BTreeSet<u32>,
    dows: BTreeSet<u32>,
    dom_restricted: bool,
    dow_restricted: bool,
}

impl CronSchedule {
    /// Parse a standard 5-field cron expression. Supports `*`, `a`, `a-b`,
    /// `a,b,c`, `*/n` and `a-b/n` in each field.
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(CronError::FieldCount(fields.len()));
        }
        let minutes = parse_field(fields[0], 0, 59)?;
        let hours = parse_field(fields[1], 0, 23)?;
        let doms = parse_field(fields[2], 1, 31)?;
        let months = parse_field(fields[3], 1, 12)?;
        // Day-of-week: 0-7 with both 0 and 7 = Sunday; normalize 7→0.
        let mut dows = parse_field(fields[4], 0, 7)?;
        if dows.remove(&7) {
            dows.insert(0);
        }
        Ok(Self {
            minutes,
            hours,
            doms,
            months,
            dows,
            dom_restricted: fields[2] != "*",
            dow_restricted: fields[4] != "*",
        })
    }

    fn day_matches(&self, dt: &DateTime<Utc>) -> bool {
        let dom_ok = self.doms.contains(&dt.day());
        let dow = dt.weekday().num_days_from_sunday(); // 0 = Sunday
        let dow_ok = self.dows.contains(&dow);
        // Vixie cron: when both day-of-month and day-of-week are restricted,
        // a match on *either* qualifies; otherwise AND with the '*' field.
        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => dom_ok || dow_ok,
            (true, false) => dom_ok,
            (false, true) => dow_ok,
            (false, false) => true,
        }
    }

    /// Smallest fire time strictly after `after` (truncated to the minute).
    /// `None` if no match within a 4-year horizon.
    pub fn next(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        // Start at the next whole minute strictly after `after`.
        let mut cur = after
            .with_second(0)?
            .with_nanosecond(0)?
            .checked_add_signed(Duration::minutes(1))?;
        let horizon = after.checked_add_signed(Duration::days(366 * 4))?;
        while cur <= horizon {
            if !self.months.contains(&cur.month()) {
                // Jump to the first day of next month, 00:00.
                cur = advance_to_next_month(cur)?;
                continue;
            }
            if !self.day_matches(&cur) {
                cur = next_midnight(cur)?;
                continue;
            }
            if !self.hours.contains(&cur.hour()) {
                cur = cur.with_minute(0)?.checked_add_signed(Duration::hours(1))?;
                continue;
            }
            if !self.minutes.contains(&cur.minute()) {
                cur = cur.checked_add_signed(Duration::minutes(1))?;
                continue;
            }
            return Some(cur);
        }
        None
    }
}

/// Advance to 00:00 on the first day of the following month.
fn advance_to_next_month(dt: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let (y, m) = if dt.month() == 12 {
        (dt.year() + 1, 1)
    } else {
        (dt.year(), dt.month() + 1)
    };
    Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0).single()
}

/// Advance to 00:00 the next day.
fn next_midnight(dt: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let midnight = dt.with_hour(0)?.with_minute(0)?.with_second(0)?.with_nanosecond(0)?;
    midnight.checked_add_signed(Duration::days(1))
}

/// Parse one cron field into the set of matching values.
fn parse_field(field: &str, min: u32, max: u32) -> Result<BTreeSet<u32>, CronError> {
    let mut out = BTreeSet::new();
    for part in field.split(',') {
        // Optional step: `range/step`.
        let (range_spec, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s.parse().map_err(|_| CronError::BadField(field.to_string()))?;
                if step == 0 {
                    return Err(CronError::BadField(field.to_string()));
                }
                (r, step)
            }
            None => (part, 1),
        };
        let (lo, hi) = if range_spec == "*" {
            (min, max)
        } else if let Some((a, b)) = range_spec.split_once('-') {
            let a: u32 = a.parse().map_err(|_| CronError::BadField(field.to_string()))?;
            let b: u32 = b.parse().map_err(|_| CronError::BadField(field.to_string()))?;
            (a, b)
        } else {
            let v: u32 = range_spec.parse().map_err(|_| CronError::BadField(field.to_string()))?;
            (v, v)
        };
        if lo < min || hi > max || lo > hi {
            return Err(CronError::BadField(field.to_string()));
        }
        let mut v = lo;
        while v <= hi {
            out.insert(v);
            v += step;
        }
    }
    if out.is_empty() {
        return Err(CronError::BadField(field.to_string()));
    }
    Ok(out)
}

/// The decision the cron controller makes for one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CronDecision {
    /// `suspend: true` — nothing scheduled.
    Suspended,
    /// No fire time elapsed since the last scheduled time.
    NotDue,
    /// A fire elapsed but is older than `starting_deadline_seconds` — skip it.
    MissedDeadline,
    /// Start a new workflow for fire time `DateTime`.
    Run(DateTime<Utc>),
    /// Forbid policy and a run is still active — skip.
    Forbidden,
    /// Replace policy — terminate `active` then start a new run.
    Replace(DateTime<Utc>),
}

/// Decide what the cron controller should do at `now`, given the last
/// scheduled fire time and the set of currently-active workflows.
pub fn evaluate(spec: &CronWorkflowSpec, status: &CronWorkflowStatus, now: DateTime<Utc>) -> CronDecision {
    if spec.suspend {
        return CronDecision::Suspended;
    }
    let schedule = match CronSchedule::parse(&spec.schedule) {
        Ok(s) => s,
        Err(_) => return CronDecision::NotDue,
    };
    // The fire time we consider is the first one after the last scheduled
    // time (or after `now - horizon` if never scheduled — use `now` minus a
    // minute so the immediately-prior fire is found).
    let anchor = status
        .last_scheduled_time
        .unwrap_or_else(|| now - Duration::days(1));
    let Some(fire) = schedule.next(anchor) else {
        return CronDecision::NotDue;
    };
    if fire > now {
        return CronDecision::NotDue;
    }
    // A fire has elapsed. Check the starting deadline.
    if let Some(deadline) = spec.starting_deadline_seconds {
        if (now - fire).num_seconds() > deadline {
            return CronDecision::MissedDeadline;
        }
    }
    let has_active = !status.active.is_empty();
    match spec.concurrency_policy {
        ConcurrencyPolicy::Allow => CronDecision::Run(fire),
        ConcurrencyPolicy::Forbid => {
            if has_active {
                CronDecision::Forbidden
            } else {
                CronDecision::Run(fire)
            }
        }
        ConcurrencyPolicy::Replace => {
            if has_active {
                CronDecision::Replace(fire)
            } else {
                CronDecision::Run(fire)
            }
        }
    }
}

/// Given finished runs `(uid, succeeded, finished_at)` newest-or-any order,
/// return the uids that should be deleted to honour the history limits.
/// `None` limit means unlimited (Argo defaults: 3 successful, 1 failed).
pub fn history_to_prune(
    runs: &[(Uuid, bool, DateTime<Utc>)],
    successful_limit: Option<u32>,
    failed_limit: Option<u32>,
) -> Vec<Uuid> {
    // Argo defaults: keep 3 successful, 1 failed.
    let succ_limit = successful_limit.unwrap_or(3) as usize;
    let fail_limit = failed_limit.unwrap_or(1) as usize;

    let mut succeeded: Vec<&(Uuid, bool, DateTime<Utc>)> = runs.iter().filter(|r| r.1).collect();
    let mut failed: Vec<&(Uuid, bool, DateTime<Utc>)> = runs.iter().filter(|r| !r.1).collect();
    // Newest first.
    succeeded.sort_by(|a, b| b.2.cmp(&a.2));
    failed.sort_by(|a, b| b.2.cmp(&a.2));

    let mut prune = Vec::new();
    prune.extend(succeeded.iter().skip(succ_limit).map(|r| r.0));
    prune.extend(failed.iter().skip(fail_limit).map(|r| r.0));
    prune
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn parse_rejects_wrong_field_count() {
        assert_eq!(CronSchedule::parse("* * *"), Err(CronError::FieldCount(3)));
        assert!(CronSchedule::parse("* * * * *").is_ok());
    }

    #[test]
    fn next_every_15_minutes() {
        let s = CronSchedule::parse("*/15 * * * *").unwrap();
        assert_eq!(s.next(t(2026, 5, 30, 10, 2)), Some(t(2026, 5, 30, 10, 15)));
        assert_eq!(s.next(t(2026, 5, 30, 10, 15)), Some(t(2026, 5, 30, 10, 30)));
        assert_eq!(s.next(t(2026, 5, 30, 10, 59)), Some(t(2026, 5, 30, 11, 0)));
    }

    #[test]
    fn next_daily_at_nine() {
        let s = CronSchedule::parse("0 9 * * *").unwrap();
        assert_eq!(s.next(t(2026, 5, 30, 8, 0)), Some(t(2026, 5, 30, 9, 0)));
        // After 9:00 same day → next day 9:00.
        assert_eq!(s.next(t(2026, 5, 30, 9, 0)), Some(t(2026, 5, 31, 9, 0)));
    }

    #[test]
    fn next_monthly_first_at_midnight() {
        let s = CronSchedule::parse("0 0 1 * *").unwrap();
        assert_eq!(s.next(t(2026, 5, 30, 12, 0)), Some(t(2026, 6, 1, 0, 0)));
    }

    #[test]
    fn next_supports_ranges_and_lists() {
        // 30th minute, hours 8 and 17, Mon-Fri.
        let s = CronSchedule::parse("30 8,17 * * 1-5").unwrap();
        // 2026-05-30 is a Saturday → skip to Monday 2026-06-01 08:30.
        assert_eq!(s.next(t(2026, 5, 30, 0, 0)), Some(t(2026, 6, 1, 8, 30)));
    }

    #[test]
    fn dow_sunday_accepts_zero_and_seven() {
        let s0 = CronSchedule::parse("0 0 * * 0").unwrap();
        let s7 = CronSchedule::parse("0 0 * * 7").unwrap();
        // 2026-05-31 is a Sunday.
        assert_eq!(s0.next(t(2026, 5, 30, 0, 0)), Some(t(2026, 5, 31, 0, 0)));
        assert_eq!(s7.next(t(2026, 5, 30, 0, 0)), Some(t(2026, 5, 31, 0, 0)));
    }

    fn spec(schedule: &str, policy: ConcurrencyPolicy) -> CronWorkflowSpec {
        CronWorkflowSpec {
            workflow_spec: WorkflowSpec::default(),
            schedule: schedule.to_string(),
            concurrency_policy: policy,
            suspend: false,
            starting_deadline_seconds: None,
            successful_jobs_history_limit: None,
            failed_jobs_history_limit: None,
            timezone: None,
        }
    }

    #[test]
    fn evaluate_suspended_returns_suspended() {
        let mut s = spec("* * * * *", ConcurrencyPolicy::Allow);
        s.suspend = true;
        assert_eq!(evaluate(&s, &CronWorkflowStatus::default(), t(2026, 5, 30, 10, 5)), CronDecision::Suspended);
    }

    #[test]
    fn evaluate_not_due_when_no_fire_elapsed() {
        let s = spec("0 9 * * *", ConcurrencyPolicy::Allow);
        let status = CronWorkflowStatus { active: vec![], last_scheduled_time: Some(t(2026, 5, 30, 9, 0)) };
        // It's 8am next consideration before the 9am fire.
        assert_eq!(evaluate(&s, &status, t(2026, 5, 30, 9, 30)), CronDecision::NotDue);
    }

    #[test]
    fn evaluate_allow_runs_even_with_active() {
        let s = spec("0 9 * * *", ConcurrencyPolicy::Allow);
        let status = CronWorkflowStatus {
            active: vec![Uuid::new_v4()],
            last_scheduled_time: Some(t(2026, 5, 29, 9, 0)),
        };
        assert_eq!(evaluate(&s, &status, t(2026, 5, 30, 9, 1)), CronDecision::Run(t(2026, 5, 30, 9, 0)));
    }

    #[test]
    fn evaluate_forbid_skips_when_active() {
        let s = spec("0 9 * * *", ConcurrencyPolicy::Forbid);
        let status = CronWorkflowStatus {
            active: vec![Uuid::new_v4()],
            last_scheduled_time: Some(t(2026, 5, 29, 9, 0)),
        };
        assert_eq!(evaluate(&s, &status, t(2026, 5, 30, 9, 1)), CronDecision::Forbidden);
    }

    #[test]
    fn evaluate_replace_when_active() {
        let s = spec("0 9 * * *", ConcurrencyPolicy::Replace);
        let status = CronWorkflowStatus {
            active: vec![Uuid::new_v4()],
            last_scheduled_time: Some(t(2026, 5, 29, 9, 0)),
        };
        assert_eq!(evaluate(&s, &status, t(2026, 5, 30, 9, 1)), CronDecision::Replace(t(2026, 5, 30, 9, 0)));
    }

    #[test]
    fn evaluate_missed_deadline_skips_late_fire() {
        let mut s = spec("0 9 * * *", ConcurrencyPolicy::Allow);
        s.starting_deadline_seconds = Some(300); // 5 minutes
        let status = CronWorkflowStatus { active: vec![], last_scheduled_time: Some(t(2026, 5, 29, 9, 0)) };
        // Fire was 9:00, but it's now 9:10 → 600s late > 300s deadline.
        assert_eq!(evaluate(&s, &status, t(2026, 5, 30, 9, 10)), CronDecision::MissedDeadline);
        // Within deadline → runs.
        assert_eq!(evaluate(&s, &status, t(2026, 5, 30, 9, 3)), CronDecision::Run(t(2026, 5, 30, 9, 0)));
    }

    #[test]
    fn history_prune_keeps_newest_within_limits() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let f1 = Uuid::new_v4();
        let f2 = Uuid::new_v4();
        let runs = vec![
            (a, true, t(2026, 5, 30, 1, 0)),
            (b, true, t(2026, 5, 30, 2, 0)),
            (c, true, t(2026, 5, 30, 3, 0)),
            (f1, false, t(2026, 5, 30, 4, 0)),
            (f2, false, t(2026, 5, 30, 5, 0)),
        ];
        // Keep 2 successful (b, c — newest) → prune a. Keep 1 failed (f2) → prune f1.
        let prune = history_to_prune(&runs, Some(2), Some(1));
        assert_eq!(prune.len(), 2);
        assert!(prune.contains(&a));
        assert!(prune.contains(&f1));
    }

    #[test]
    fn concurrency_policy_serde_roundtrip() {
        assert_eq!(serde_json::to_string(&ConcurrencyPolicy::Forbid).unwrap(), "\"Forbid\"");
        let p: ConcurrencyPolicy = serde_json::from_str("\"Replace\"").unwrap();
        assert_eq!(p, ConcurrencyPolicy::Replace);
        assert_eq!(ConcurrencyPolicy::default(), ConcurrencyPolicy::Allow);
    }
}
