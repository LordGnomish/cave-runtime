//! CronJob — real 5-field cron expression parser + concurrencyPolicy
//! state machine + suspend.
//!
//! Mirrors `pkg/controller/cronjob/utils.go::nextScheduleTime` plus the
//! `vendor/github.com/robfig/cron/v3` parser. We implement the standard
//! POSIX `m h dom mon dow` form with `*`, `*/N`, comma lists, and
//! ranges (`a-b`). DOW is 0-6 with `0=Sun` (matching upstream).
//!
//! `next_after(t)` returns the first scheduled instant *strictly after* `t`.

use crate::types::{Cite, ControllerError, TenantId};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CronError {
    #[error("expected 5 fields separated by whitespace, got {0}")]
    WrongFieldCount(usize),
    #[error("field `{field}` value `{token}` out of range {min}..={max}")]
    OutOfRange { field: &'static str, token: String, min: u32, max: u32 },
    #[error("field `{field}` value `{token}` is not parseable")]
    Unparsable { field: &'static str, token: String },
    #[error("step value must be > 0 in `{0}`")]
    BadStep(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronExpression {
    pub minute: Vec<u32>,      // 0..60
    pub hour: Vec<u32>,        // 0..24
    pub day_of_month: Vec<u32>, // 1..32
    pub month: Vec<u32>,       // 1..13
    pub day_of_week: Vec<u32>, // 0..7
}

impl CronExpression {
    /// Parse a 5-field cron string.
    pub fn parse(s: &str) -> Result<Self, CronError> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(CronError::WrongFieldCount(parts.len()));
        }
        Ok(Self {
            minute: expand_field("minute", parts[0], 0, 59)?,
            hour: expand_field("hour", parts[1], 0, 23)?,
            day_of_month: expand_field("day_of_month", parts[2], 1, 31)?,
            month: expand_field("month", parts[3], 1, 12)?,
            day_of_week: expand_field("day_of_week", parts[4], 0, 6)?,
        })
    }

    /// True iff `t` matches every field. (DOM/DOW use OR semantics when both
    /// are restricted — same as cron.)
    pub fn matches(&self, t: DateTime<Utc>) -> bool {
        if !self.minute.contains(&(t.minute() as u32)) {
            return false;
        }
        if !self.hour.contains(&(t.hour() as u32)) {
            return false;
        }
        if !self.month.contains(&(t.month())) {
            return false;
        }
        let dom_full = self.day_of_month.len() == 31;
        let dow_full = self.day_of_week.len() == 7;
        let dom_match = self.day_of_month.contains(&(t.day()));
        let dow = (t.weekday().num_days_from_sunday()) as u32;
        let dow_match = self.day_of_week.contains(&dow);
        match (dom_full, dow_full) {
            (false, false) => dom_match || dow_match,
            (true, false) => dow_match,
            (false, true) => dom_match,
            (true, true) => true,
        }
    }

    /// First minute boundary strictly after `from` whose all fields match.
    pub fn next_after(&self, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
        // Truncate to minute, then advance one minute and step until match.
        let truncated = Utc
            .with_ymd_and_hms(from.year(), from.month(), from.day(), from.hour(), from.minute(), 0)
            .single()?;
        let mut cursor = truncated + Duration::minutes(1);
        // Cap the search at 4 years to bound the worst case.
        let limit = cursor + Duration::days(366 * 4);
        while cursor < limit {
            if self.matches(cursor) {
                return Some(cursor);
            }
            cursor += Duration::minutes(1);
        }
        None
    }
}

fn expand_field(
    field: &'static str,
    token: &str,
    min: u32,
    max: u32,
) -> Result<Vec<u32>, CronError> {
    let mut out = std::collections::BTreeSet::new();
    for part in token.split(',') {
        // Step form: `range/step` or `*/step`
        let (range_part, step) = match part.split_once('/') {
            Some((r, s)) => {
                let s: u32 = s.parse().map_err(|_| CronError::Unparsable { field, token: token.into() })?;
                if s == 0 {
                    return Err(CronError::BadStep(token.into()));
                }
                (r, s)
            }
            None => (part, 1),
        };
        let (lo, hi) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let a: u32 = a.parse().map_err(|_| CronError::Unparsable { field, token: token.into() })?;
            let b: u32 = b.parse().map_err(|_| CronError::Unparsable { field, token: token.into() })?;
            (a, b)
        } else {
            let n: u32 = range_part
                .parse()
                .map_err(|_| CronError::Unparsable { field, token: token.into() })?;
            (n, n)
        };
        if lo < min || hi > max || lo > hi {
            return Err(CronError::OutOfRange { field, token: token.into(), min, max });
        }
        let mut v = lo;
        while v <= hi {
            out.insert(v);
            v += step;
        }
    }
    Ok(out.into_iter().collect())
}

// ── concurrencyPolicy state machine ──────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConcurrencyPolicy {
    Allow,
    Forbid,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronJobSpec {
    pub name: String,
    pub tenant: TenantId,
    pub schedule: String,
    pub concurrency: ConcurrencyPolicy,
    pub suspended: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronJobStatus {
    pub last_schedule_time: Option<DateTime<Utc>>,
    pub active_job_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CronAction {
    NoOp,
    Launch { name: String },
    Replace { delete: Vec<String>, then_launch: String },
}

/// Decide what to do at instant `now`. Mirrors `syncCronJob` upstream.
pub fn decide(
    spec: &CronJobSpec,
    status: &CronJobStatus,
    now: DateTime<Utc>,
    caller: &TenantId,
) -> Result<CronAction, ControllerError> {
    if caller != &spec.tenant {
        return Err(ControllerError::TenantDenied {
            tenant: caller.clone(),
            kind: "CronJob",
            name: spec.name.clone(),
        });
    }
    if spec.suspended {
        return Ok(CronAction::NoOp);
    }
    let expr = CronExpression::parse(&spec.schedule).map_err(|e| ControllerError::InvalidSpec {
        kind: "CronJob",
        reason: e.to_string(),
    })?;
    // Has a scheduled instant elapsed since `last_schedule_time` (or the start
    // of this minute, whichever is older)?
    let from = status.last_schedule_time.unwrap_or(now - Duration::minutes(1));
    let next = match expr.next_after(from) {
        Some(t) => t,
        None => return Ok(CronAction::NoOp),
    };
    if next > now {
        return Ok(CronAction::NoOp);
    }
    // A fire is due. Apply the concurrency policy.
    match spec.concurrency {
        ConcurrencyPolicy::Allow => Ok(CronAction::Launch {
            name: format!("{}-{}", spec.name, next.timestamp()),
        }),
        ConcurrencyPolicy::Forbid if !status.active_job_names.is_empty() => Ok(CronAction::NoOp),
        ConcurrencyPolicy::Forbid => Ok(CronAction::Launch {
            name: format!("{}-{}", spec.name, next.timestamp()),
        }),
        ConcurrencyPolicy::Replace => Ok(CronAction::Replace {
            delete: status.active_job_names.clone(),
            then_launch: format!("{}-{}", spec.name, next.timestamp()),
        }),
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/cronjob/utils.go", "ParseSchedule");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn dt(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    #[test]
    fn parse_star_expands_to_full_range() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "expandField",
            "tenant-cron-star"
        );
        let e = CronExpression::parse("* * * * *").unwrap();
        assert_eq!(e.minute.len(), 60);
        assert_eq!(e.hour.len(), 24);
        assert_eq!(e.day_of_month.len(), 31);
        assert_eq!(e.month.len(), 12);
        assert_eq!(e.day_of_week.len(), 7);
    }

    #[test]
    fn parse_step_form_picks_every_nth() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "expandField",
            "tenant-cron-step"
        );
        let e = CronExpression::parse("*/15 * * * *").unwrap();
        assert_eq!(e.minute, vec![0, 15, 30, 45]);
    }

    #[test]
    fn parse_range_and_list_combine() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "expandField",
            "tenant-cron-range-list"
        );
        let e = CronExpression::parse("0 9-11,17 * * *").unwrap();
        assert_eq!(e.minute, vec![0]);
        assert_eq!(e.hour, vec![9, 10, 11, 17]);
    }

    #[test]
    fn parse_rejects_wrong_field_count() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "ParseSchedule",
            "tenant-cron-fields"
        );
        assert!(matches!(
            CronExpression::parse("* * * *").unwrap_err(),
            CronError::WrongFieldCount(4)
        ));
    }

    #[test]
    fn parse_rejects_out_of_range() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "expandField",
            "tenant-cron-oor"
        );
        assert!(matches!(
            CronExpression::parse("60 * * * *").unwrap_err(),
            CronError::OutOfRange { .. }
        ));
    }

    #[test]
    fn parse_rejects_zero_step() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "expandField",
            "tenant-cron-step-zero"
        );
        assert!(matches!(
            CronExpression::parse("*/0 * * * *").unwrap_err(),
            CronError::BadStep(_)
        ));
    }

    #[test]
    fn matches_handles_dom_dow_or_semantics() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "matches",
            "tenant-cron-dom-dow-or"
        );
        // "0 0 1 * 1" — at midnight, on day-1 OR Monday.
        let e = CronExpression::parse("0 0 1 * 1").unwrap();
        // 2026-04-01 is a Wednesday; matches because dom=1.
        assert!(e.matches(dt(2026, 4, 1, 0, 0)));
        // 2026-04-06 is a Monday; matches because dow=1.
        assert!(e.matches(dt(2026, 4, 6, 0, 0)));
        // 2026-04-02 is Thursday and not day 1 — must not match.
        assert!(!e.matches(dt(2026, 4, 2, 0, 0)));
    }

    #[test]
    fn next_after_returns_next_minute_boundary() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "nextScheduleTime",
            "tenant-cron-next"
        );
        let e = CronExpression::parse("*/5 * * * *").unwrap();
        let now = dt(2026, 4, 26, 10, 7);
        // next */5 strictly after 10:07 is 10:10.
        assert_eq!(e.next_after(now).unwrap(), dt(2026, 4, 26, 10, 10));
    }

    #[test]
    fn forbid_skips_when_active_job_present() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "acme"
        );
        let spec = CronJobSpec {
            name: "report".into(),
            tenant: TenantId::new("acme"),
            schedule: "* * * * *".into(),
            concurrency: ConcurrencyPolicy::Forbid,
            suspended: false,
        };
        let status = CronJobStatus { active_job_names: vec!["report-old".into()], ..Default::default() };
        // Now is one minute past midnight to ensure a fire is due.
        assert_eq!(decide(&spec, &status, dt(2026, 4, 26, 0, 1), &tenant).unwrap(), CronAction::NoOp);
    }

    #[test]
    fn replace_emits_delete_then_launch() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "acme"
        );
        let spec = CronJobSpec {
            name: "report".into(),
            tenant: TenantId::new("acme"),
            schedule: "* * * * *".into(),
            concurrency: ConcurrencyPolicy::Replace,
            suspended: false,
        };
        let status = CronJobStatus { active_job_names: vec!["a".into(), "b".into()], ..Default::default() };
        let action = decide(&spec, &status, dt(2026, 4, 26, 0, 1), &tenant).unwrap();
        match action {
            CronAction::Replace { delete, then_launch } => {
                assert_eq!(delete, vec!["a".to_string(), "b".to_string()]);
                assert!(then_launch.starts_with("report-"));
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn suspended_cronjob_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "acme"
        );
        let spec = CronJobSpec {
            name: "report".into(),
            tenant: TenantId::new("acme"),
            schedule: "* * * * *".into(),
            concurrency: ConcurrencyPolicy::Allow,
            suspended: true,
        };
        let status = CronJobStatus::default();
        assert_eq!(decide(&spec, &status, dt(2026, 4, 26, 0, 1), &tenant).unwrap(), CronAction::NoOp);
    }

    #[test]
    fn cross_tenant_caller_is_refused() {
        let (_cite, attacker) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "tenantCheck",
            "tenant-attacker"
        );
        let spec = CronJobSpec {
            name: "report".into(),
            tenant: TenantId::new("acme"),
            schedule: "* * * * *".into(),
            concurrency: ConcurrencyPolicy::Allow,
            suspended: false,
        };
        let err = decide(&spec, &CronJobStatus::default(), dt(2026, 4, 26, 0, 1), &attacker).unwrap_err();
        assert!(matches!(err, ControllerError::TenantDenied { .. }));
    }
}
