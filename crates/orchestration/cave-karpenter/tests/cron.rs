// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// RED→GREEN cycle 8 (continuation ray #3): port of robfig/cron/v3
// ParseStandard + SpecSchedule.Next, the cron engine Karpenter's
// Budget.IsActive (pkg/apis/v1/nodepool.go) delegates to. Standard 5-field
// cron (minute hour dom month dow) in UTC, with the TZ= prefix Karpenter adds.
//
// Reference instants (UTC):
//   2026-01-01 00:00:00 = 1767225600  (a Thursday; weekday 4, Sun=0)

use cave_karpenter::cron::{parse_standard, CronSchedule};

const T_2026_01_01_000000: i64 = 1_767_225_600;
const DAY: i64 = 86_400;

fn next(spec: &str, after: i64) -> i64 {
    parse_standard(spec)
        .expect("valid cron")
        .next(after)
        .expect("a next time exists")
}

// ── parsing ──────────────────────────────────────────────────────────────────

#[test]
fn parse_accepts_standard_five_fields() {
    assert!(parse_standard("* * * * *").is_ok());
    assert!(parse_standard("0 0 * * *").is_ok());
    assert!(parse_standard("*/15 * * * *").is_ok());
    assert!(parse_standard("30 9 * * 1-5").is_ok());
    assert!(parse_standard("0 0 1 * *").is_ok());
}

#[test]
fn parse_accepts_month_and_weekday_names() {
    assert!(parse_standard("0 0 1 JAN *").is_ok());
    assert!(parse_standard("0 0 * * SUN").is_ok());
    assert!(parse_standard("0 0 * jan-mar mon-fri").is_ok());
}

#[test]
fn parse_accepts_tz_prefix() {
    // Karpenter formats the schedule as "TZ=UTC <schedule>"
    assert!(parse_standard("TZ=UTC 0 0 * * *").is_ok());
}

#[test]
fn parse_rejects_bad_input() {
    assert!(parse_standard("").is_err());
    assert!(parse_standard("* * * *").is_err()); // 4 fields
    assert!(parse_standard("* * * * * *").is_err()); // 6 fields
    assert!(parse_standard("60 * * * *").is_err()); // minute out of range
    assert!(parse_standard("* 24 * * *").is_err()); // hour out of range
    assert!(parse_standard("* * * FOO *").is_err()); // bad month name
}

// ── Next: every-minute / strictly-after ──────────────────────────────────────

#[test]
fn next_every_minute_is_strictly_after() {
    assert_eq!(
        next("* * * * *", T_2026_01_01_000000),
        T_2026_01_01_000000 + 60
    );
    // exactly on a boundary still advances (strictly greater than input)
    assert_eq!(
        next("* * * * *", T_2026_01_01_000000 + 60),
        T_2026_01_01_000000 + 120
    );
}

// ── Next: daily / sub-day ────────────────────────────────────────────────────

#[test]
fn next_daily_midnight() {
    assert_eq!(
        next("0 0 * * *", T_2026_01_01_000000),
        T_2026_01_01_000000 + DAY
    );
}

#[test]
fn next_every_fifteen_minutes() {
    assert_eq!(
        next("*/15 * * * *", T_2026_01_01_000000),
        T_2026_01_01_000000 + 900
    );
}

#[test]
fn next_specific_time_same_day() {
    // 09:30 on the same Thursday
    assert_eq!(
        next("30 9 * * *", T_2026_01_01_000000),
        T_2026_01_01_000000 + 9 * 3600 + 30 * 60
    );
}

// ── Next: weekday constraint ─────────────────────────────────────────────────

#[test]
fn next_weekday_range_hits_same_thursday() {
    // 2026-01-01 is a Thursday (in Mon-Fri = 1-5), so 09:30 lands same day
    assert_eq!(
        next("30 9 * * 1-5", T_2026_01_01_000000),
        T_2026_01_01_000000 + 9 * 3600 + 30 * 60
    );
}

#[test]
fn next_sunday_skips_to_first_sunday() {
    // From Thu 2026-01-01, next Sunday 00:00 is 2026-01-04 (+3 days)
    assert_eq!(
        next("0 0 * * SUN", T_2026_01_01_000000),
        T_2026_01_01_000000 + 3 * DAY
    );
}

// ── Next: month rollover ─────────────────────────────────────────────────────

#[test]
fn next_first_of_month_rolls_to_february() {
    // From 2026-01-01 00:00, next "1st 00:00" is 2026-02-01 (Jan has 31 days)
    assert_eq!(
        next("0 0 1 * *", T_2026_01_01_000000),
        T_2026_01_01_000000 + 31 * DAY
    );
}

#[test]
fn next_dom_dow_union_when_neither_is_star() {
    // "0 0 13 * 5" → day-of-month 13 OR Friday (robfig OR semantics).
    // From 2026-01-01 (Thu), the first Friday is 2026-01-02 (+1 day) at 00:00,
    // which is earlier than the 13th — so the union picks the Friday.
    let s: CronSchedule = parse_standard("0 0 13 * 5").unwrap();
    assert_eq!(
        s.next(T_2026_01_01_000000).unwrap(),
        T_2026_01_01_000000 + DAY
    );
}
