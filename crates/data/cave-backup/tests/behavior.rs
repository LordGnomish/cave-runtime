// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD: Velero `pkg/controller/schedule_controller.go::getNextRunTime` cron port.
//!
//! Upstream: vmware-tanzu/velero — the schedule controller delegates next-fire
//! computation to a robfig/cron parser's `Next(after)`. Cave ports the matching
//! semantics in `schedule::CronSchedule` (step `*/N`, range `lo-hi`, and the
//! standard DOM/DOW OR rule). The existing `tests/schedule_tdd.rs` only exercises
//! the all-wildcard daily (`0 0 * * *`) and hourly (`0 * * * *`) expressions, so
//! the step / range / day-of-month-OR-day-of-week branches of `parse_field` and
//! `CronSchedule::matches` are otherwise untested. These cases drive the public
//! `cave_backup::schedule::next_run` entry point directly.

use chrono::{TimeZone, Utc};

/// `*/15 * * * *` matches minutes {0,15,30,45}. From 10:07 the candidate scan
/// starts at 10:08 and the next member of the set is 10:15.
#[test]
fn next_run_step_minute_field_advances_to_next_multiple() {
    let after = Utc.with_ymd_and_hms(2026, 5, 29, 10, 7, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 5, 29, 10, 15, 0).unwrap();
    let got = cave_backup::schedule::next_run("*/15 * * * *", after).unwrap();
    assert_eq!(got, expected);
}

/// Same `*/15` schedule, but past the final in-hour slot (45): from 10:50 the
/// next firing rolls into the following hour at minute 0 (11:00).
#[test]
fn next_run_step_minute_field_rolls_to_next_hour() {
    let after = Utc.with_ymd_and_hms(2026, 5, 29, 10, 50, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 5, 29, 11, 0, 0).unwrap();
    let got = cave_backup::schedule::next_run("*/15 * * * *", after).unwrap();
    assert_eq!(got, expected);
}

/// `0 9-17 * * *` = minute 0 of the hours 9..=17 (inclusive range). From 08:30
/// the next firing is the same day at 09:00 — exercises the `lo-hi` parse branch.
#[test]
fn next_run_range_hour_field_same_day() {
    let after = Utc.with_ymd_and_hms(2026, 5, 29, 8, 30, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 5, 29, 9, 0, 0).unwrap();
    let got = cave_backup::schedule::next_run("0 9-17 * * *", after).unwrap();
    assert_eq!(got, expected);
}

/// `0 9-17 * * *`: from 17:30 there is no remaining in-range hour today
/// (hours 18..=23 are outside 9-17), so the next firing rolls to the following
/// day at the bottom of the range, 09:00.
#[test]
fn next_run_range_hour_field_rolls_to_next_day() {
    let after = Utc.with_ymd_and_hms(2026, 5, 29, 17, 30, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 5, 30, 9, 0, 0).unwrap();
    let got = cave_backup::schedule::next_run("0 9-17 * * *", after).unwrap();
    assert_eq!(got, expected);
}

/// `0 0 1 * 0` = 00:00 on the 1st of the month OR any Sunday (both DOM and DOW
/// restricted => OR per standard cron). From Fri 2026-05-29 12:00 the first
/// match is via the day-of-week arm: Sun 2026-05-31 00:00 (the 31st, not the
/// 1st), proving the OR fires on a Sunday that is not the 1st.
#[test]
fn next_run_dom_dow_or_fires_on_sunday() {
    let after = Utc.with_ymd_and_hms(2026, 5, 29, 12, 0, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 5, 31, 0, 0, 0).unwrap();
    let got = cave_backup::schedule::next_run("0 0 1 * 0", after).unwrap();
    assert_eq!(got, expected);
}

/// Same `0 0 1 * 0` schedule via the day-of-month arm of the OR: starting from
/// Sun 2026-05-31 12:00, the next match is Mon 2026-06-01 00:00 — the 1st of the
/// month, which is NOT a Sunday, proving the DOM arm fires independently of DOW.
#[test]
fn next_run_dom_dow_or_fires_on_first_of_month() {
    let after = Utc.with_ymd_and_hms(2026, 5, 31, 12, 0, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
    let got = cave_backup::schedule::next_run("0 0 1 * 0", after).unwrap();
    assert_eq!(got, expected);
}
