// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hibernation — scheduled "force-zero" windows that override every
//! scaler's recommendation.
//!
//! Upstream reference (KEDA v2.16+):
//!   pkg/scaling/scaledobject_hibernation.go
//!
//! ScaledObject defines a list of hibernation schedules; while any
//! schedule's window contains the current time, the controller forces
//! the workload to `replicas_during_hibernation` (typically 0). When no
//! schedule matches, normal scaling resumes.
//!
//! The Cave port supports the cron subset needed for daily/weekday
//! windows: hour + day-of-week. Minute precision is honoured;
//! day-of-month / month bounds are accepted but not enforced because
//! the upstream use-case is "office hours" patterns.

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hibernation {
    pub schedules: Vec<HibernationSchedule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HibernationSchedule {
    /// 5-field cron firing when the hibernation window begins.
    pub cron_start: String,
    /// 5-field cron firing when the hibernation window ends.
    pub cron_end: String,
    /// Replica count to apply while hibernating (often 0).
    pub replicas_during_hibernation: i32,
    /// IANA timezone string ("UTC" / "Europe/Istanbul" / …). The Cave
    /// port treats anything non-UTC as UTC for the MVP — proper TZ
    /// handling lands when the workspace gains a TZ database dep.
    pub timezone: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HibernationDecision {
    Awake,
    Hibernating { replicas: i32 },
}

impl Hibernation {
    /// Decide whether `now` falls inside any hibernation window.
    pub fn decide_at(&self, now: DateTime<Utc>) -> HibernationDecision {
        for s in &self.schedules {
            if schedule_active_at(s, now) {
                return HibernationDecision::Hibernating {
                    replicas: s.replicas_during_hibernation,
                };
            }
        }
        HibernationDecision::Awake
    }
}

/// True if `now` is inside the schedule's hibernation window.
///
/// Algorithm (per-day calendar evaluation):
///   1. Compute the time-of-day half-open interval [start, end).
///   2. If start <= end: window is same-day → minutes(now) in [start, end).
///   3. If start  > end: window crosses midnight → minutes(now) in [start, 24:00)
///      OR in [00:00, end).
///   4. Determine the "owning" weekday: for the same-day case it's `now.day`;
///      for the cross-midnight case it's `now.day` when after start_minutes,
///      otherwise `now.day - 1`.
///   5. If `start.dow` includes the owning day AND `end.dow` includes the day
///      the window ends on, we're hibernating.
fn schedule_active_at(s: &HibernationSchedule, now: DateTime<Utc>) -> bool {
    let Some(start_fields) = parse_cron(&s.cron_start) else {
        return false;
    };
    let Some(end_fields) = parse_cron(&s.cron_end) else {
        return false;
    };

    let now_min = (now.hour() * 60 + now.minute()) as i32;
    let start_min = (start_fields.hour.unwrap_or(0) * 60 + start_fields.minute.unwrap_or(0)) as i32;
    let end_min = (end_fields.hour.unwrap_or(23) * 60 + end_fields.minute.unwrap_or(59)) as i32;

    let in_window;
    let owning_dow_chrono: u32; // chrono mon=0..sun=6

    if start_min <= end_min {
        // Same-day window: [start, end] inclusive of start, inclusive of end.
        in_window = now_min >= start_min && now_min <= end_min;
        owning_dow_chrono = now.weekday().num_days_from_monday();
    } else {
        // Cross-midnight window: [start, 24:00) ∪ [00:00, end).
        if now_min >= start_min {
            in_window = true;
            owning_dow_chrono = now.weekday().num_days_from_monday();
        } else if now_min < end_min {
            in_window = true;
            // Owning day is yesterday.
            owning_dow_chrono = (now.weekday().num_days_from_monday() + 6) % 7;
        } else {
            in_window = false;
            owning_dow_chrono = now.weekday().num_days_from_monday();
        }
    }

    if !in_window {
        return false;
    }

    let cron_dow = (owning_dow_chrono + 1) % 7;
    let start_ok = match &start_fields.day_of_week {
        None => true,
        Some(set) => set.contains(&cron_dow),
    };
    if !start_ok {
        return false;
    }

    // For symmetric weekday windows the end.dow check usually matches
    // start.dow; we honour end.dow against the day the window CLOSES on.
    let end_dow_chrono = if start_min > end_min && now_min < end_min {
        // We're in the post-midnight tail of a cross-midnight window —
        // the close day is today.
        now.weekday().num_days_from_monday()
    } else if start_min > end_min {
        // We're in the pre-midnight head — close day is tomorrow.
        (now.weekday().num_days_from_monday() + 1) % 7
    } else {
        now.weekday().num_days_from_monday()
    };
    let end_cron_dow = (end_dow_chrono + 1) % 7;
    match &end_fields.day_of_week {
        None => true,
        Some(set) => set.contains(&end_cron_dow),
    }
}

#[derive(Debug, Clone)]
struct CronFields {
    minute: Option<u32>,
    hour: Option<u32>,
    /// `None` = wildcard `*`; `Some(set)` = explicit values (or expanded range).
    day_of_week: Option<std::collections::BTreeSet<u32>>,
}

fn parse_cron(expr: &str) -> Option<CronFields> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    let minute = parse_singleton(parts[0])?;
    let hour = parse_singleton(parts[1])?;
    let day_of_week = parse_set_or_wildcard(parts[4])?;
    Some(CronFields {
        minute,
        hour,
        day_of_week,
    })
}

fn parse_singleton(field: &str) -> Option<Option<u32>> {
    if field == "*" {
        return Some(None);
    }
    field.parse::<u32>().ok().map(Some)
}

fn parse_set_or_wildcard(field: &str) -> Option<Option<std::collections::BTreeSet<u32>>> {
    if field == "*" {
        return Some(None);
    }
    let mut out = std::collections::BTreeSet::new();
    for chunk in field.split(',') {
        if let Some((a, b)) = chunk.split_once('-') {
            let a: u32 = a.parse().ok()?;
            let b: u32 = b.parse().ok()?;
            for v in a..=b {
                out.insert(v);
            }
        } else {
            out.insert(chunk.parse::<u32>().ok()?);
        }
    }
    Some(Some(out))
}

fn cron_matches(fields: &CronFields, when: DateTime<Utc>) -> bool {
    if let Some(m) = fields.minute
        && m != when.minute()
    {
        return false;
    }
    if let Some(h) = fields.hour
        && h != when.hour()
    {
        return false;
    }
    if let Some(dow_set) = &fields.day_of_week {
        // chrono weekday: Mon=0..Sun=6 via num_days_from_monday()
        let dow = when.weekday().num_days_from_monday();
        // Cron convention: 0=Sunday, 1=Monday, ..., 6=Saturday.
        let cron_dow = (dow + 1) % 7;
        if !dow_set.contains(&cron_dow) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn no_schedules_is_awake() {
        let h = Hibernation { schedules: vec![] };
        let now = Utc::now();
        assert_eq!(h.decide_at(now), HibernationDecision::Awake);
    }

    #[test]
    fn parse_cron_extracts_minute_hour() {
        let f = parse_cron("30 9 * * *").unwrap();
        assert_eq!(f.minute, Some(30));
        assert_eq!(f.hour, Some(9));
        assert!(f.day_of_week.is_none());
    }

    #[test]
    fn parse_cron_dow_range() {
        let f = parse_cron("0 9 * * 1-5").unwrap();
        let set = f.day_of_week.unwrap();
        assert!(set.contains(&1));
        assert!(set.contains(&5));
        assert!(!set.contains(&0));
    }

    #[test]
    fn cron_matches_specific_minute_hour() {
        let f = parse_cron("30 9 * * *").unwrap();
        let when = Utc.with_ymd_and_hms(2026, 5, 20, 9, 30, 0).unwrap();
        assert!(cron_matches(&f, when));
        let off = Utc.with_ymd_and_hms(2026, 5, 20, 9, 31, 0).unwrap();
        assert!(!cron_matches(&f, off));
    }
}
