// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CronJob controller — schedules `Job` objects on a cron expression.
//!
//! Upstream: [`pkg/controller/cronjob`] (the v2 controller). The full
//! controller deals with concurrency policy, deadline, time-zone strings,
//! and starting-deadline-seconds. This scaffold only validates the schedule
//! string shape and decides whether a fire is due.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConcurrencyPolicy {
    Allow,
    Forbid,
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobSpec {
    pub name: String,
    pub namespace: String,
    /// Standard 5-field cron expression (`m h dom mon dow`). Validation here
    /// is intentionally minimal — the full parser lives upstream in
    /// `vendor/github.com/robfig/cron/v3`.
    pub schedule: String,
    pub concurrency: ConcurrencyPolicy,
    pub suspended: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronJobStatus {
    pub last_schedule_time: Option<DateTime<Utc>>,
    pub active_jobs: u32,
}

/// Returns `Err` if the schedule string is structurally invalid (not 5
/// whitespace-separated tokens). Mirrors the entry-point validation in
/// `pkg/controller/cronjob/utils.go::ParseSchedule`.
pub fn validate_schedule(schedule: &str) -> Result<(), ControllerError> {
    let tokens: Vec<&str> = schedule.split_whitespace().collect();
    if tokens.len() != 5 {
        return Err(ControllerError::InvalidSpec {
            kind: "CronJob",
            reason: format!("schedule must have 5 fields, got {}", tokens.len()),
        });
    }
    Ok(())
}

/// Mirrors `syncCronJob` — decides whether the controller should fire a new
/// Job in this pass.
pub fn reconcile(
    spec: &CronJobSpec,
    status: &CronJobStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    validate_schedule(&spec.schedule)?;
    if spec.suspended {
        return Ok(Reconcile::NoOp);
    }
    match spec.concurrency {
        ConcurrencyPolicy::Forbid if status.active_jobs > 0 => Ok(Reconcile::NoOp),
        ConcurrencyPolicy::Replace if status.active_jobs > 0 => {
            Ok(Reconcile::Delete(status.active_jobs))
        }
        _ => Ok(Reconcile::Create(1)),
    }
}

// ── Cron expression evaluator ────────────────────────────────────────────────
//
// Five-field standard cron: `minute hour dom month dow`.
// Each field admits:
//   * `*`            — every value in the field's natural range
//   * `N`            — single value
//   * `N-M`          — inclusive range
//   * `*/S` / `N-M/S` — every Sth value within the (sub)range
//   * `a,b,c`        — comma-separated list of any of the above
//
// `?` is accepted as a synonym for `*` in dom/dow (some cron dialects use
// it to mean "no specific value"); upstream `robfig/cron/v3` treats them
// the same way at parse time so we follow suit.

/// Structured error from cron parsing / evaluation. Mirrors the error
/// surface of `robfig/cron/v3::ParseStandard` (used upstream by
/// `pkg/controller/cronjob/utils.go::ParseStandardWithOptions`).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScheduleError {
    /// Field count differed from 5.
    #[error("cron expression must have 5 fields, got {got}")]
    WrongFieldCount { got: usize },
    /// Field could not be parsed (non-numeric, malformed range, …).
    #[error("invalid field {field}: {token}")]
    InvalidField { field: &'static str, token: String },
    /// Field value is outside the legal range for that field.
    #[error("out-of-range value in field {field}: {value} not in {min}..={max}")]
    OutOfRange {
        field: &'static str,
        value: i32,
        min: i32,
        max: i32,
    },
    /// Schedule will never fire (e.g. `0 0 31 2 *` — Feb 31).
    #[error("schedule {schedule:?} can never fire (no calendar match within search window)")]
    Unsatisfiable { schedule: String },
}

#[derive(Debug, Clone)]
struct CronSchedule {
    minute: Vec<u32>,  // sorted, 0..=59
    hour: Vec<u32>,    // sorted, 0..=23
    dom: Vec<u32>,     // sorted, 1..=31
    month: Vec<u32>,   // sorted, 1..=12
    dow: Vec<u32>,     // sorted, 0..=6 (Sun=0)
    dom_was_star: bool,
    dow_was_star: bool,
}

const FIELD_NAMES: [&str; 5] = ["minute", "hour", "dom", "month", "dow"];

fn parse_schedule(s: &str) -> Result<CronSchedule, ScheduleError> {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    if tokens.len() != 5 {
        return Err(ScheduleError::WrongFieldCount { got: tokens.len() });
    }
    let dom_was_star = tokens[2] == "*" || tokens[2] == "?";
    let dow_was_star = tokens[4] == "*" || tokens[4] == "?";
    let minute = parse_field(tokens[0], FIELD_NAMES[0], 0, 59)?;
    let hour = parse_field(tokens[1], FIELD_NAMES[1], 0, 23)?;
    let dom = parse_field(tokens[2], FIELD_NAMES[2], 1, 31)?;
    let month = parse_field(tokens[3], FIELD_NAMES[3], 1, 12)?;
    let dow = parse_field(tokens[4], FIELD_NAMES[4], 0, 6)?;
    Ok(CronSchedule {
        minute,
        hour,
        dom,
        month,
        dow,
        dom_was_star,
        dow_was_star,
    })
}

fn parse_field(
    token: &str,
    field: &'static str,
    min: i32,
    max: i32,
) -> Result<Vec<u32>, ScheduleError> {
    // `?` is treated as `*` per robfig/cron parity.
    if token == "*" || token == "?" {
        return Ok((min..=max).map(|v| v as u32).collect());
    }
    let mut out: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for part in token.split(',') {
        // Step?
        let (range_part, step) = if let Some((r, s)) = part.split_once('/') {
            let step: i32 = s.parse().map_err(|_| ScheduleError::InvalidField {
                field,
                token: part.to_string(),
            })?;
            if step <= 0 {
                return Err(ScheduleError::InvalidField {
                    field,
                    token: part.to_string(),
                });
            }
            (r, step)
        } else {
            (part, 1)
        };
        let (lo, hi) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let a: i32 = a.parse().map_err(|_| ScheduleError::InvalidField {
                field,
                token: part.to_string(),
            })?;
            let b: i32 = b.parse().map_err(|_| ScheduleError::InvalidField {
                field,
                token: part.to_string(),
            })?;
            (a, b)
        } else {
            let v: i32 = range_part
                .parse()
                .map_err(|_| ScheduleError::InvalidField {
                    field,
                    token: part.to_string(),
                })?;
            if step != 1 {
                // `N/S` is shorthand for `N-max/S` in robfig/cron.
                (v, max)
            } else {
                (v, v)
            }
        };
        if lo < min || lo > max || hi < min || hi > max || lo > hi {
            return Err(ScheduleError::OutOfRange {
                field,
                value: if lo < min || lo > max { lo } else { hi },
                min,
                max,
            });
        }
        let mut v = lo;
        while v <= hi {
            out.insert(v as u32);
            v += step;
        }
    }
    Ok(out.into_iter().collect())
}

impl CronSchedule {
    /// Does `t` exactly match this schedule?
    fn matches(&self, t: DateTime<Utc>) -> bool {
        let minute = t.minute();
        let hour = t.hour();
        let dom = t.day();
        let month = t.month();
        // chrono: Mon=1 .. Sun=7 (from `weekday().number_from_monday()`).
        // Cron uses Sun=0..Sat=6 — convert.
        let dow = t.weekday().num_days_from_sunday();
        if !self.minute.contains(&minute) {
            return false;
        }
        if !self.hour.contains(&hour) {
            return false;
        }
        if !self.month.contains(&month) {
            return false;
        }
        // Day-of-month / day-of-week dual constraint. Per Vixie cron + robfig
        // semantics, if BOTH dom and dow are constrained (not `*`), match if
        // EITHER holds (OR). If only one is constrained, only that one
        // applies. If both are `*`, both are trivially true.
        let dom_match = self.dom.contains(&dom);
        let dow_match = self.dow.contains(&dow);
        match (self.dom_was_star, self.dow_was_star) {
            (true, true) => true,
            (false, true) => dom_match,
            (true, false) => dow_match,
            (false, false) => dom_match || dow_match,
        }
    }
}

/// Most recent fire time `t` such that `t <= now` AND (`last.is_none()` OR
/// `t > last`). Mirrors `pkg/controller/cronjob/utils.go::nextScheduleTime`'s
/// search semantics (which delegate to `cron.Schedule.Next` / `Prev` walks).
///
/// Returns `Ok(None)` if no such `t` exists within a bounded look-back
/// window (we cap at ~5 years to stay deterministic; upstream caps at
/// `100 * 366 * 24 * 60` minutes as a safety net).
///
/// Returns `Err(ScheduleError::Unsatisfiable)` if the schedule cannot fire
/// at all within the window (e.g. `59 23 31 2 *` — Feb 31).
pub fn next_schedule_time(
    schedule: &str,
    last: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, ScheduleError> {
    let parsed = parse_schedule(schedule)?;
    // Search starts at `now` truncated to the minute (cron minimum granularity)
    // and walks backward minute-by-minute. Capped at ~5 years.
    const LOOK_BACK_MIN: i64 = 5 * 366 * 24 * 60;
    let mut t = Utc
        .with_ymd_and_hms(
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
            0,
        )
        .single()
        .unwrap_or(now);
    // Returned time must be STRICTLY > last and ≤ now-truncated. When no
    // last is given, the lower bound is the start of the look-back window.
    let lower_bound_exclusive = last;
    let look_back_floor = t - Duration::minutes(LOOK_BACK_MIN);
    let mut steps = 0i64;
    loop {
        // Bail-out on the exclusive lower bound (last).
        if let Some(lb) = lower_bound_exclusive {
            if t <= lb {
                return Ok(None);
            }
        }
        if t < look_back_floor {
            // Walked the full window without a match — schedule is
            // unsatisfiable in the calendar (e.g. Feb 31).
            return Err(ScheduleError::Unsatisfiable {
                schedule: schedule.to_string(),
            });
        }
        if parsed.matches(t) {
            return Ok(Some(t));
        }
        t = t - Duration::minutes(1);
        steps += 1;
        if steps > LOOK_BACK_MIN {
            return Err(ScheduleError::Unsatisfiable {
                schedule: schedule.to_string(),
            });
        }
    }
}

/// Convenience wrapper used by the (legacy) reconciler — returns the next
/// fire time strictly after `now`. Implemented in terms of
/// [`next_schedule_time`] (which finds the most-recent fire), advancing
/// by the field-granularity until a match is found.
///
/// Charter v2: replaces the previous `unimplemented!` stub.
pub fn next_fire_time(
    spec: &CronJobSpec,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, ControllerError> {
    let parsed = parse_schedule(&spec.schedule).map_err(|e| ControllerError::InvalidSpec {
        kind: "CronJob",
        reason: e.to_string(),
    })?;
    // Walk forward, capped at ~5 years.
    const LOOK_AHEAD_MIN: i64 = 5 * 366 * 24 * 60;
    let mut t = Utc
        .with_ymd_and_hms(
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute(),
            0,
        )
        .single()
        .unwrap_or(now)
        + Duration::minutes(1);
    for _ in 0..LOOK_AHEAD_MIN {
        if parsed.matches(t) {
            return Ok(t);
        }
        t = t + Duration::minutes(1);
    }
    Err(ControllerError::InvalidSpec {
        kind: "CronJob",
        reason: format!("schedule {:?} never fires within look-ahead window", spec.schedule),
    })
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/cronjob/cronjob_controllerv2.go", "ControllerV2");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn cj(schedule: &str, policy: ConcurrencyPolicy, suspended: bool) -> CronJobSpec {
        CronJobSpec {
            name: "report".into(),
            namespace: "default".into(),
            schedule: schedule.into(),
            concurrency: policy,
            suspended,
        }
    }

    #[test]
    fn validates_five_field_cron_expression() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "ParseSchedule",
            "tenant-cron-validate"
        );
        let _ = tenant;
        assert!(validate_schedule("*/5 * * * *").is_ok());
        assert!(validate_schedule("bad").is_err());
        assert!(validate_schedule("* * * *").is_err());
    }

    #[test]
    fn forbid_skips_when_already_active() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cron-forbid"
        );
        let s = cj("0 * * * *", ConcurrencyPolicy::Forbid, false);
        let st = CronJobStatus { active_jobs: 1, ..Default::default() };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::NoOp);
    }

    #[test]
    fn replace_deletes_active_then_recreates() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cron-replace"
        );
        let s = cj("0 * * * *", ConcurrencyPolicy::Replace, false);
        let st = CronJobStatus { active_jobs: 2, ..Default::default() };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Delete(2));
    }

    #[test]
    fn suspended_cronjob_does_nothing() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cron-suspended"
        );
        let s = cj("0 * * * *", ConcurrencyPolicy::Allow, true);
        let st = CronJobStatus::default();
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::NoOp);
    }
}
