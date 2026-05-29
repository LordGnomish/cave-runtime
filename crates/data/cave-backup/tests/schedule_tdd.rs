// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD: Velero schedule_controller getNextRunTime port — next_run from cron.

use chrono::{TimeZone, Utc};

#[test]
fn schedule_next_run_after_known_time() {
    // "0 0 * * *" = daily at midnight UTC.
    // After 2026-05-29T10:00:00Z the next firing is 2026-05-30T00:00:00Z.
    let after = Utc.with_ymd_and_hms(2026, 5, 29, 10, 0, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 5, 30, 0, 0, 0).unwrap();

    let got = cave_backup::schedule::next_run("0 0 * * *", after).unwrap();
    assert_eq!(got, expected);
}

#[test]
fn schedule_next_run_hourly() {
    let after = Utc.with_ymd_and_hms(2026, 5, 29, 10, 30, 0).unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 5, 29, 11, 0, 0).unwrap();
    let got = cave_backup::schedule::next_run("0 * * * *", after).unwrap();
    assert_eq!(got, expected);
}
