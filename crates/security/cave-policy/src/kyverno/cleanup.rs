// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kyverno CleanupPolicy schedule + resource selection.
//!
//! Upstream: kyverno/kyverno v1.18.1 —
//!   - api/kyverno/v2/cleanup_policy_types.go (CleanupPolicy / ClusterCleanupPolicy)
//!   - pkg/controllers/cleanup/controller.go (cron schedule + candidate select)
//!
//! This module ports the **pure** parts of the cleanup controller:
//!   - a 5-field cron schedule (`CronSchedule`) used to decide when a sweep runs,
//!   - `selects_for_cleanup`, which decides whether a single resource is a
//!     deletion candidate (match − exclude, gated on `conditions`).
//!
//! The reconciler loop itself (k8s informer list + delete API calls + status
//! patching) remains scope_cut to the Phase-2 controller-runtime port.

use super::models::CleanupPolicy;
use super::validate::eval_conditions;
use super::{matches_exclude, matches_resources};
use crate::error::PolicyError;
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use serde_json::Value;

/// A parsed standard 5-field cron schedule: `minute hour dom month dow`.
#[derive(Debug, Clone)]
pub struct CronSchedule {
    minutes: FieldSet,
    hours: FieldSet,
    days_of_month: FieldSet,
    months: FieldSet,
    days_of_week: FieldSet,
    /// True when both day-of-month and day-of-week are restricted; standard
    /// cron treats that as an OR between the two day fields.
    dom_and_dow_restricted: bool,
}

#[derive(Debug, Clone)]
struct FieldSet {
    allowed: Vec<u32>,
    /// `*` (unrestricted) — matches any value in range.
    wildcard: bool,
}

impl FieldSet {
    fn contains(&self, v: u32) -> bool {
        self.wildcard || self.allowed.contains(&v)
    }
}

impl CronSchedule {
    /// Parse a 5-field cron expression. Each field supports `*`, `*/step`,
    /// `a-b` ranges, `a,b,c` lists, and bare values.
    pub fn parse(expr: &str) -> Result<CronSchedule, PolicyError> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(PolicyError::Parse(format!(
                "cron expression must have 5 fields, got {}: {expr:?}",
                fields.len()
            )));
        }
        let minutes = parse_field(fields[0], 0, 59)?;
        let hours = parse_field(fields[1], 0, 23)?;
        let days_of_month = parse_field(fields[2], 1, 31)?;
        let months = parse_field(fields[3], 1, 12)?;
        let days_of_week = parse_field(fields[4], 0, 7)?; // 0 and 7 = Sunday

        let dom_and_dow_restricted = !days_of_month.wildcard && !days_of_week.wildcard;
        Ok(CronSchedule {
            minutes,
            hours,
            days_of_month,
            months,
            days_of_week,
            dom_and_dow_restricted,
        })
    }

    /// Does this schedule fire at the given (second-truncated) instant?
    pub fn matches(&self, dt: &DateTime<Utc>) -> bool {
        if !self.minutes.contains(dt.minute()) || !self.hours.contains(dt.hour()) {
            return false;
        }
        if !self.months.contains(dt.month()) {
            return false;
        }
        // cron weekday: Sunday is both 0 and 7.
        let dow = dt.weekday().num_days_from_sunday();
        let dow_match = self.days_of_week.contains(dow)
            || (dow == 0 && self.days_of_week.contains(7));
        let dom_match = self.days_of_month.contains(dt.day());

        let day_match = if self.dom_and_dow_restricted {
            // Both restricted → OR (classic Vixie cron semantics).
            dom_match || dow_match
        } else {
            dom_match && dow_match
        };
        day_match
    }

    /// First firing strictly after `from`. Searches minute-by-minute up to a
    /// bounded horizon (~4 years) to cover Feb-29-only schedules.
    pub fn next_after(&self, from: &DateTime<Utc>) -> Option<DateTime<Utc>> {
        // Start at the next whole minute after `from`.
        let mut t = (*from + Duration::minutes(1))
            .with_second(0)?
            .with_nanosecond(0)?;
        let horizon = *from + Duration::days(366 * 4);
        while t <= horizon {
            if self.matches(&t) {
                return Some(t);
            }
            t += Duration::minutes(1);
        }
        None
    }
}

fn parse_field(field: &str, min: u32, max: u32) -> Result<FieldSet, PolicyError> {
    if field == "*" {
        return Ok(FieldSet {
            allowed: vec![],
            wildcard: true,
        });
    }
    let mut allowed = Vec::new();
    for part in field.split(',') {
        // step form: "*/n" or "a-b/n"
        let (range_part, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s
                    .parse()
                    .map_err(|_| PolicyError::Parse(format!("invalid cron step: {s:?}")))?;
                if step == 0 {
                    return Err(PolicyError::Parse("cron step cannot be 0".into()));
                }
                (r, step)
            }
            None => (part, 1),
        };

        let (lo, hi) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            (parse_num(a, min, max)?, parse_num(b, min, max)?)
        } else {
            let v = parse_num(range_part, min, max)?;
            (v, v)
        };
        if lo > hi {
            return Err(PolicyError::Parse(format!(
                "cron range start {lo} > end {hi}"
            )));
        }
        let mut v = lo;
        while v <= hi {
            allowed.push(v);
            v += step;
        }
    }
    allowed.sort_unstable();
    allowed.dedup();
    Ok(FieldSet {
        allowed,
        wildcard: false,
    })
}

fn parse_num(s: &str, min: u32, max: u32) -> Result<u32, PolicyError> {
    let v: u32 = s
        .trim()
        .parse()
        .map_err(|_| PolicyError::Parse(format!("invalid cron value: {s:?}")))?;
    if v < min || v > max {
        return Err(PolicyError::Parse(format!(
            "cron value {v} out of range {min}..={max}"
        )));
    }
    Ok(v)
}

/// Is `resource` a deletion candidate for `policy`?
///
/// True iff the resource is in scope of `spec.match`, NOT excluded by
/// `spec.exclude`, and satisfies `spec.conditions` (absent = unconditional).
/// `context` is the evaluation context (`{ "request": { "object": ... } }`)
/// used to resolve `{{ ... }}` references in the conditions.
pub fn selects_for_cleanup(
    policy: &CleanupPolicy,
    resource: &Value,
    namespace: Option<&str>,
    context: &Value,
) -> bool {
    if !matches_resources(&policy.spec.match_resources, resource, namespace, "DELETE") {
        return false;
    }
    if let Some(exclude) = &policy.spec.exclude {
        if matches_exclude(exclude, resource, namespace, "DELETE") {
            return false;
        }
    }
    if let Some(conditions) = &policy.spec.conditions {
        match eval_conditions(conditions, resource, context) {
            Ok(true) => {}
            _ => return false,
        }
    }
    true
}
