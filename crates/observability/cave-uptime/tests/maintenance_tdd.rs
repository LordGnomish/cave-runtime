// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD test for the maintenance-window evaluation engine.
//!
//! Line-ported from upstream Uptime Kuma `server/model/maintenance.js`
//! (`calcDuration`, `getStatus`, `getDayOfWeekList`, recurring-strategy
//! window evaluation). Persistence (BeanModel/redbean) and socket fan-out
//! stay out of crate; only the pure time/calendar evaluation is ported.

use cave_uptime::maintenance::{
    Maintenance, MaintenanceStatus, MaintenanceStrategy, TimeRange,
};
use chrono::{TimeZone, Utc};

/// `calcDuration`: end_time - start_time in seconds, +24h when crossing midnight.
#[test]
fn calc_duration_same_day() {
    let m = Maintenance::recurring_weekday(
        "deploy",
        TimeRange::new("02:00", "04:00"),
        vec![1, 3, 5],
    );
    // 02:00 -> 04:00 == 7200 seconds
    assert_eq!(m.calc_duration(), 7200);
}

#[test]
fn calc_duration_crosses_midnight() {
    let m = Maintenance::recurring_weekday(
        "night",
        TimeRange::new("23:00", "01:00"),
        vec![6],
    );
    // 23:00 -> 01:00 crosses midnight -> 2h == 7200 (upstream adds 24*3600)
    assert_eq!(m.calc_duration(), 7200);
}

/// `getStatus`: inactive maintenance is always "inactive".
#[test]
fn status_inactive_when_disabled() {
    let mut m = Maintenance::manual("manual win");
    m.active = false;
    let now = Utc::now();
    assert_eq!(m.get_status(now), MaintenanceStatus::Inactive);
}

/// `getStatus`: active manual maintenance is always "under-maintenance".
#[test]
fn status_manual_is_under_maintenance() {
    let m = Maintenance::manual("manual win");
    let now = Utc::now();
    assert_eq!(m.get_status(now), MaintenanceStatus::UnderMaintenance);
}

/// `getStatus`: single window before its start_date is "scheduled".
#[test]
fn status_single_before_start_is_scheduled() {
    let start = Utc.with_ymd_and_hms(2026, 7, 1, 10, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let m = Maintenance::single("release", start, end);
    let now = Utc.with_ymd_and_hms(2026, 6, 30, 9, 0, 0).unwrap();
    assert_eq!(m.get_status(now), MaintenanceStatus::Scheduled);
}

/// `getStatus`: single window after its end_date is "ended".
#[test]
fn status_single_after_end_is_ended() {
    let start = Utc.with_ymd_and_hms(2026, 7, 1, 10, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let m = Maintenance::single("release", start, end);
    let now = Utc.with_ymd_and_hms(2026, 7, 2, 9, 0, 0).unwrap();
    assert_eq!(m.get_status(now), MaintenanceStatus::Ended);
}

/// `getStatus`: single window within [start,end] is "under-maintenance".
#[test]
fn status_single_in_window_is_under_maintenance() {
    let start = Utc.with_ymd_and_hms(2026, 7, 1, 10, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let m = Maintenance::single("release", start, end);
    let now = Utc.with_ymd_and_hms(2026, 7, 1, 11, 0, 0).unwrap();
    assert_eq!(m.get_status(now), MaintenanceStatus::UnderMaintenance);
    assert!(m.is_under_maintenance(now));
}

/// Recurring-weekday: active inside the daily window on a listed weekday.
#[test]
fn recurring_weekday_active_inside_window() {
    // 2026-07-01 is a Wednesday (weekday 3, ISO Mon=1).
    let m = Maintenance::recurring_weekday(
        "midday",
        TimeRange::new("11:00", "13:00"),
        vec![3], // Wednesday only
    );
    let now = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    assert!(m.is_active_at(now));
    assert_eq!(m.get_status(now), MaintenanceStatus::UnderMaintenance);
}

/// Recurring-weekday: not active on a weekday not in the list.
#[test]
fn recurring_weekday_inactive_on_unlisted_day() {
    // 2026-07-02 is a Thursday (weekday 4).
    let m = Maintenance::recurring_weekday(
        "midday",
        TimeRange::new("11:00", "13:00"),
        vec![3], // Wednesday only
    );
    let now = Utc.with_ymd_and_hms(2026, 7, 2, 12, 0, 0).unwrap();
    assert!(!m.is_active_at(now));
}

/// Recurring-weekday: not active outside the daily time window.
#[test]
fn recurring_weekday_inactive_outside_time() {
    let m = Maintenance::recurring_weekday(
        "midday",
        TimeRange::new("11:00", "13:00"),
        vec![3],
    );
    let now = Utc.with_ymd_and_hms(2026, 7, 1, 9, 0, 0).unwrap();
    assert!(!m.is_active_at(now));
}

/// Recurring-weekday window that crosses midnight is active just after midnight
/// on the day *following* a listed weekday.
#[test]
fn recurring_weekday_crosses_midnight() {
    // Window 23:00 -> 01:00 listed for Wednesday (3).
    let m = Maintenance::recurring_weekday(
        "night",
        TimeRange::new("23:00", "01:00"),
        vec![3],
    );
    // 23:30 Wednesday -> active.
    let wed_night = Utc.with_ymd_and_hms(2026, 7, 1, 23, 30, 0).unwrap();
    assert!(m.is_active_at(wed_night));
    // 00:30 Thursday -> still inside the Wednesday window.
    let thu_morning = Utc.with_ymd_and_hms(2026, 7, 2, 0, 30, 0).unwrap();
    assert!(m.is_active_at(thu_morning));
}

/// Recurring-day-of-month: active on a listed day of the month.
#[test]
fn recurring_day_of_month_active() {
    let m = Maintenance::recurring_day_of_month(
        "monthly",
        TimeRange::new("01:00", "02:00"),
        vec![15],
    );
    let now = Utc.with_ymd_and_hms(2026, 7, 15, 1, 30, 0).unwrap();
    assert!(m.is_active_at(now));
}

#[test]
fn recurring_day_of_month_inactive_other_day() {
    let m = Maintenance::recurring_day_of_month(
        "monthly",
        TimeRange::new("01:00", "02:00"),
        vec![15],
    );
    let now = Utc.with_ymd_and_hms(2026, 7, 16, 1, 30, 0).unwrap();
    assert!(!m.is_active_at(now));
}

/// `getDayOfWeekList`: weekday list is sorted ascending.
#[test]
fn weekday_list_sorted() {
    let m = Maintenance::recurring_weekday(
        "x",
        TimeRange::new("00:00", "01:00"),
        vec![5, 1, 3],
    );
    assert_eq!(m.day_of_week_list(), vec![1, 3, 5]);
}

/// Strategy is reflected by the constructor.
#[test]
fn strategy_constructors() {
    assert_eq!(Maintenance::manual("a").strategy, MaintenanceStrategy::Manual);
    let s = Maintenance::single(
        "b",
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 1, 1, 1, 0, 0).unwrap(),
    );
    assert_eq!(s.strategy, MaintenanceStrategy::Single);
}
