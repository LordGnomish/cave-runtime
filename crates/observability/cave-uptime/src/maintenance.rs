// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Maintenance-window evaluation engine.
//!
//! Line-ported from upstream Uptime Kuma `server/model/maintenance.js`
//! (the `Maintenance` BeanModel). This module ports the pure, in-crate
//! time/calendar evaluation logic:
//!
//! * `calcDuration`         -> [`Maintenance::calc_duration`]
//! * `getStatus`            -> [`Maintenance::get_status`]
//! * `isUnderMaintenance`   -> [`Maintenance::is_under_maintenance`]
//! * `getDayOfWeekList`     -> [`Maintenance::day_of_week_list`]
//! * `getDayOfMonthList`    -> [`Maintenance::day_of_month_list`]
//! * recurring-strategy window evaluation (the `recurring-interval` /
//!   `recurring-weekday` / `recurring-day-of-month` cron generation in
//!   `generateCron` collapsed into a direct in-window check
//!   [`Maintenance::is_active_at`]).
//!
//! Persistence (redbean / `R.store`), the croner job scheduler, timezone
//! dropdown plumbing, and the socket fan-out (`sendMaintenanceListByUserID`)
//! stay out of crate — those belong to cave-rdbms / cave-net / cave-portal.
//! All evaluation here runs against UTC, matching upstream's `dayjs.utc`
//! pathway used in `calcDuration`.

use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use serde::{Deserialize, Serialize};

/// Maintenance strategy, mirroring upstream `this.strategy`.
///
/// Upstream string values: `"manual"`, `"single"`, `"recurring-interval"`,
/// `"recurring-weekday"`, `"recurring-day-of-month"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenanceStrategy {
    Manual,
    Single,
    RecurringInterval,
    RecurringWeekday,
    RecurringDayOfMonth,
}

/// Maintenance status, mirroring the strings returned by upstream `getStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenanceStatus {
    /// Maintenance is disabled (`!this.active`).
    Inactive,
    /// Window has a future `start_date`.
    Scheduled,
    /// Window's `end_date` has passed.
    Ended,
    /// Window is currently active.
    UnderMaintenance,
    /// Active but cannot be evaluated.
    Unknown,
}

/// A daily start/end clock range expressed as `HH:MM` (24h), matching
/// upstream `this.start_time` / `this.end_time`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    /// Seconds-from-midnight of the start time.
    pub start_secs: u32,
    /// Seconds-from-midnight of the end time.
    pub end_secs: u32,
}

impl TimeRange {
    /// Parse a `HH:MM` pair into a [`TimeRange`]. Mirrors upstream's
    /// `this.start_time.split(":")` parsing in `generateCron`.
    pub fn new(start: &str, end: &str) -> Self {
        TimeRange {
            start_secs: parse_hhmm(start),
            end_secs: parse_hhmm(end),
        }
    }
}

/// Parse `HH:MM` to seconds-from-midnight. Faithful to upstream
/// `parseInt(array[0])` / `parseInt(array[1])` minute/hour parsing.
fn parse_hhmm(s: &str) -> u32 {
    let mut it = s.split(':');
    let hour: u32 = it.next().and_then(|h| h.trim().parse().ok()).unwrap_or(0);
    let minute: u32 = it.next().and_then(|m| m.trim().parse().ok()).unwrap_or(0);
    hour * 3600 + minute * 60
}

/// A maintenance window.
///
/// Field names mirror the upstream bean columns where practical.
#[derive(Debug, Clone)]
pub struct Maintenance {
    pub title: String,
    pub strategy: MaintenanceStrategy,
    /// Upstream `this.active` (`!!this.active`).
    pub active: bool,
    /// Upstream `this.start_date` — bounds the overall window.
    pub start_date: Option<DateTime<Utc>>,
    /// Upstream `this.end_date` — bounds the overall window.
    pub end_date: Option<DateTime<Utc>>,
    /// Daily clock range for recurring strategies (`start_time`/`end_time`).
    pub time_range: Option<TimeRange>,
    /// Upstream `this.weekdays` (ISO weekday numbers, Mon=1..Sun=7).
    pub weekdays: Vec<u32>,
    /// Upstream `this.days_of_month` (1..=31).
    pub days_of_month: Vec<u32>,
    /// Upstream `this.interval_day` for `recurring-interval`.
    pub interval_day: u32,
}

impl Maintenance {
    fn base(title: &str, strategy: MaintenanceStrategy) -> Self {
        Maintenance {
            title: title.to_string(),
            strategy,
            active: true,
            start_date: None,
            end_date: None,
            time_range: None,
            weekdays: Vec::new(),
            days_of_month: Vec::new(),
            interval_day: 1,
        }
    }

    /// `strategy === "manual"`.
    pub fn manual(title: &str) -> Self {
        Self::base(title, MaintenanceStrategy::Manual)
    }

    /// `strategy === "single"` — a one-shot window bounded by [start, end].
    pub fn single(title: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        let mut m = Self::base(title, MaintenanceStrategy::Single);
        m.start_date = Some(start);
        m.end_date = Some(end);
        m
    }

    /// `strategy === "recurring-weekday"`.
    pub fn recurring_weekday(title: &str, time_range: TimeRange, weekdays: Vec<u32>) -> Self {
        let mut m = Self::base(title, MaintenanceStrategy::RecurringWeekday);
        m.time_range = Some(time_range);
        m.weekdays = weekdays;
        m
    }

    /// `strategy === "recurring-day-of-month"`.
    pub fn recurring_day_of_month(
        title: &str,
        time_range: TimeRange,
        days_of_month: Vec<u32>,
    ) -> Self {
        let mut m = Self::base(title, MaintenanceStrategy::RecurringDayOfMonth);
        m.time_range = Some(time_range);
        m.days_of_month = days_of_month;
        m
    }

    /// `strategy === "recurring-interval"` — every `interval_day` days.
    pub fn recurring_interval(title: &str, time_range: TimeRange, interval_day: u32) -> Self {
        let mut m = Self::base(title, MaintenanceStrategy::RecurringInterval);
        m.time_range = Some(time_range);
        m.interval_day = interval_day.max(1);
        m
    }

    /// Port of upstream `getDayOfWeekList`: sorted ascending.
    pub fn day_of_week_list(&self) -> Vec<u32> {
        let mut v = self.weekdays.clone();
        v.sort_unstable();
        v
    }

    /// Port of upstream `getDayOfMonthList`: sorted ascending.
    pub fn day_of_month_list(&self) -> Vec<u32> {
        let mut v = self.days_of_month.clone();
        v.sort_unstable();
        v
    }

    /// Port of upstream `calcDuration`:
    /// ```js
    /// let duration = dayjs.utc(end_time,"HH:mm").diff(dayjs.utc(start_time,"HH:mm"),"second");
    /// if (duration < 0) { duration += 24 * 3600; }
    /// ```
    pub fn calc_duration(&self) -> i64 {
        let Some(tr) = &self.time_range else {
            return 0;
        };
        let mut duration = tr.end_secs as i64 - tr.start_secs as i64;
        if duration < 0 {
            duration += 24 * 3600;
        }
        duration
    }

    /// Port of upstream `getStatus(now)`.
    ///
    /// ```js
    /// if (!this.active) return "inactive";
    /// if (strategy === "manual") return "under-maintenance";
    /// if (start_date && now.isBefore(start_date)) return "scheduled";
    /// if (end_date && now.isAfter(end_date)) return "ended";
    /// if (strategy === "single") return "under-maintenance";
    /// // recurring: derived from active-window check
    /// ```
    pub fn get_status(&self, now: DateTime<Utc>) -> MaintenanceStatus {
        if !self.active {
            return MaintenanceStatus::Inactive;
        }

        if self.strategy == MaintenanceStrategy::Manual {
            return MaintenanceStatus::UnderMaintenance;
        }

        // Window not yet started.
        if let Some(start) = self.start_date {
            if now < start {
                return MaintenanceStatus::Scheduled;
            }
        }

        // Window already ended.
        if let Some(end) = self.end_date {
            if now > end {
                return MaintenanceStatus::Ended;
            }
        }

        if self.strategy == MaintenanceStrategy::Single {
            return MaintenanceStatus::UnderMaintenance;
        }

        // Recurring strategies: collapse upstream's cron `beanMeta.status`
        // (scheduled / under-maintenance) into a direct window check.
        if self.is_active_at(now) {
            MaintenanceStatus::UnderMaintenance
        } else {
            MaintenanceStatus::Scheduled
        }
    }

    /// Port of upstream `isUnderMaintenance`:
    /// `(await this.getStatus()) === "under-maintenance"`.
    pub fn is_under_maintenance(&self, now: DateTime<Utc>) -> bool {
        self.get_status(now) == MaintenanceStatus::UnderMaintenance
    }

    /// Direct evaluation of whether `now` falls inside a recurring (or single)
    /// maintenance window. This collapses the croner schedule that upstream
    /// builds in `generateCron` + `getRunningTimeslot` into a deterministic
    /// in-window predicate.
    ///
    /// The daily window runs from `start_secs` to `end_secs`; when the window
    /// crosses midnight (`end < start`, the `calcDuration` +24h case) it is
    /// open from `start_secs` through end-of-day and again from start-of-day
    /// through `end_secs` of the *next* calendar day — matching the duration
    /// semantics of upstream's timeslot.
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        // Respect overall [start_date, end_date] bounds first.
        if let Some(start) = self.start_date {
            if now < start {
                return false;
            }
        }
        if let Some(end) = self.end_date {
            if now > end {
                return false;
            }
        }

        match self.strategy {
            MaintenanceStrategy::Manual => true,
            MaintenanceStrategy::Single => {
                // For single windows the [start,end] bounds above are the window.
                self.start_date.is_some() && self.end_date.is_some()
            }
            MaintenanceStrategy::RecurringWeekday => {
                self.in_recurring_window(now, |m, day_start| {
                    let wd = iso_weekday(day_start.weekday());
                    m.weekdays.contains(&wd)
                })
            }
            MaintenanceStrategy::RecurringDayOfMonth => {
                self.in_recurring_window(now, |m, day_start| {
                    m.days_of_month.contains(&day_start.day())
                })
            }
            MaintenanceStrategy::RecurringInterval => {
                // Every `interval_day` days. Without an anchor date upstream
                // keys off the cron `*/N` field; we treat any day as eligible
                // when interval_day == 1, otherwise key off day-of-month modulo.
                self.in_recurring_window(now, |m, day_start| {
                    m.interval_day <= 1 || (day_start.day() - 1) % m.interval_day == 0
                })
            }
        }
    }

    /// Shared recurring-window evaluation. `day_eligible` decides whether the
    /// calendar day that *opens* the window qualifies (listed weekday / day of
    /// month / interval day). Handles the cross-midnight (+24h) case.
    fn in_recurring_window<F>(&self, now: DateTime<Utc>, day_eligible: F) -> bool
    where
        F: Fn(&Self, DateTime<Utc>) -> bool,
    {
        let Some(tr) = &self.time_range else {
            return false;
        };
        let secs_today = now.num_seconds_from_midnight();
        let crosses_midnight = tr.end_secs < tr.start_secs;

        // Case A: same-day window [start, end) on an eligible day.
        if !crosses_midnight {
            if secs_today >= tr.start_secs
                && secs_today < tr.end_secs
                && day_eligible(self, now)
            {
                return true;
            }
            return false;
        }

        // Case B: window crosses midnight.
        // B1: late part of an eligible day's window  [start, 24:00).
        if secs_today >= tr.start_secs && day_eligible(self, now) {
            return true;
        }
        // B2: early part [00:00, end) belonging to the *previous* day's window.
        if secs_today < tr.end_secs {
            let yesterday = now - chrono::Duration::days(1);
            if day_eligible(self, yesterday) {
                return true;
            }
        }
        false
    }
}

/// Map a chrono [`Weekday`] to upstream's ISO numbering (Mon=1 .. Sun=7).
fn iso_weekday(wd: Weekday) -> u32 {
    wd.number_from_monday()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parse_hhmm_basic() {
        assert_eq!(parse_hhmm("00:00"), 0);
        assert_eq!(parse_hhmm("01:30"), 5400);
        assert_eq!(parse_hhmm("23:59"), 23 * 3600 + 59 * 60);
    }

    #[test]
    fn duration_zero_when_no_time_range() {
        assert_eq!(Maintenance::manual("m").calc_duration(), 0);
    }

    #[test]
    fn iso_weekday_mapping() {
        // 2026-07-01 is a Wednesday.
        let d = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();
        assert_eq!(iso_weekday(d.weekday()), 3);
    }
}
