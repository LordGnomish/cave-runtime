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
    pub fn parse(_expr: &str) -> Result<Self, CronError> {
        unimplemented!()
    }

    /// Smallest fire time strictly after `after` (truncated to the minute).
    /// `None` if no match within a 4-year horizon.
    pub fn next(&self, _after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        unimplemented!()
    }
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
pub fn evaluate(_spec: &CronWorkflowSpec, _status: &CronWorkflowStatus, _now: DateTime<Utc>) -> CronDecision {
    unimplemented!()
}

/// Given finished runs `(uid, succeeded, finished_at)` newest-or-any order,
/// return the uids that should be deleted to honour the history limits.
/// `None` limit means unlimited (Argo defaults: 3 successful, 1 failed).
pub fn history_to_prune(
    _runs: &[(Uuid, bool, DateTime<Utc>)],
    _successful_limit: Option<u32>,
    _failed_limit: Option<u32>,
) -> Vec<Uuid> {
    unimplemented!()
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
