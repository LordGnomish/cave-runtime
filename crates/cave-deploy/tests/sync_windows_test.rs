// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of ArgoCD sync windows — upstream
//! `pkg/apis/application/v1alpha1/types.go` `SyncWindows.{Active,CanSync,
//! Matches}` + the robfig/cron schedule evaluation used by
//! `controller/state.go` to gate auto-sync inside maintenance windows.
//!
//! cave-deploy already carries the `SyncWindow` CRD shape (src/rbac.rs); this
//! ports the pure-Rust cron + window-activeness algebra (no subprocess, no
//! external cron crate).

use cave_deploy::rbac::{SyncWindow, SyncWindowKind};
use cave_deploy::sync_windows::{
    can_sync, matching_windows, parse_duration, window_active, CronSchedule,
};
use chrono::{TimeZone, Utc};

fn win(
    kind: SyncWindowKind,
    schedule: &str,
    duration: &str,
    manual: bool,
    apps: &[&str],
    namespaces: &[&str],
    clusters: &[&str],
) -> SyncWindow {
    SyncWindow {
        kind,
        schedule: schedule.to_string(),
        duration: duration.to_string(),
        applications: apps.iter().map(|s| s.to_string()).collect(),
        namespaces: namespaces.iter().map(|s| s.to_string()).collect(),
        clusters: clusters.iter().map(|s| s.to_string()).collect(),
        manual_sync: manual,
        time_zone: None,
    }
}

// ─── cron parsing + next() ──────────────────────────────────────────────────

#[test]
fn cron_parse_rejects_wrong_field_count() {
    assert!(CronSchedule::parse("0 10 * *").is_err());
    assert!(CronSchedule::parse("* * * * *").is_ok());
}

#[test]
fn cron_next_daily_at_ten() {
    let s = CronSchedule::parse("0 10 * * *").unwrap();
    // strictly after 09:30 → today 10:00
    let after = Utc.with_ymd_and_hms(2026, 5, 30, 9, 30, 0).unwrap();
    let next = s.next(after).unwrap();
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 5, 30, 10, 0, 0).unwrap());
    // strictly after 10:30 → tomorrow 10:00
    let after2 = Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap();
    let next2 = s.next(after2).unwrap();
    assert_eq!(next2, Utc.with_ymd_and_hms(2026, 5, 31, 10, 0, 0).unwrap());
}

#[test]
fn cron_step_and_range_and_list() {
    // every 15 minutes
    let s = CronSchedule::parse("*/15 * * * *").unwrap();
    let after = Utc.with_ymd_and_hms(2026, 5, 30, 9, 1, 0).unwrap();
    assert_eq!(
        s.next(after).unwrap(),
        Utc.with_ymd_and_hms(2026, 5, 30, 9, 15, 0).unwrap()
    );
    // hour range 9-17, list of minutes
    let s2 = CronSchedule::parse("0,30 9-17 * * *").unwrap();
    let after2 = Utc.with_ymd_and_hms(2026, 5, 30, 17, 31, 0).unwrap();
    // next valid is tomorrow 09:00
    assert_eq!(
        s2.next(after2).unwrap(),
        Utc.with_ymd_and_hms(2026, 5, 31, 9, 0, 0).unwrap()
    );
}

#[test]
fn cron_day_of_week() {
    // Mondays only (dow=1) at midnight. 2026-05-30 is a Saturday.
    let s = CronSchedule::parse("0 0 * * 1").unwrap();
    let after = Utc.with_ymd_and_hms(2026, 5, 30, 12, 0, 0).unwrap();
    let next = s.next(after).unwrap();
    // next Monday is 2026-06-01
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap());
}

// ─── duration parsing (Go time.ParseDuration subset) ────────────────────────

#[test]
fn parse_duration_units() {
    assert_eq!(parse_duration("1h").unwrap().num_minutes(), 60);
    assert_eq!(parse_duration("30m").unwrap().num_minutes(), 30);
    assert_eq!(parse_duration("2h30m").unwrap().num_minutes(), 150);
    assert_eq!(parse_duration("45s").unwrap().num_seconds(), 45);
    assert!(parse_duration("bogus").is_none());
}

// ─── window active() ────────────────────────────────────────────────────────

#[test]
fn window_active_within_duration() {
    let w = win(SyncWindowKind::Allow, "0 10 * * *", "1h", false, &[], &[], &[]);
    let inside = Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap();
    let outside = Utc.with_ymd_and_hms(2026, 5, 30, 11, 30, 0).unwrap();
    assert!(window_active(&w, inside));
    assert!(!window_active(&w, outside));
}

// ─── CanSync precedence ─────────────────────────────────────────────────────

#[test]
fn can_sync_no_windows_allows() {
    let now = Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap();
    assert!(can_sync(&[], now, false));
}

#[test]
fn can_sync_active_deny_blocks_auto_but_manual_if_enabled() {
    let now = Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap();
    // active deny window, manualSync=true → auto blocked, manual allowed
    let deny = win(SyncWindowKind::Deny, "0 10 * * *", "1h", true, &[], &[], &[]);
    assert!(!can_sync(std::slice::from_ref(&deny), now, false));
    assert!(can_sync(std::slice::from_ref(&deny), now, true));

    // active deny, manualSync=false → both blocked
    let deny2 = win(SyncWindowKind::Deny, "0 10 * * *", "1h", false, &[], &[], &[]);
    assert!(!can_sync(std::slice::from_ref(&deny2), now, false));
    assert!(!can_sync(std::slice::from_ref(&deny2), now, true));
}

#[test]
fn can_sync_active_allow_permits() {
    let now = Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap();
    let allow = win(SyncWindowKind::Allow, "0 10 * * *", "1h", false, &[], &[], &[]);
    assert!(can_sync(std::slice::from_ref(&allow), now, false));
}

#[test]
fn can_sync_inactive_allow_blocks_outside_window() {
    // an allow window that is NOT active right now → syncs blocked outside it
    let now = Utc.with_ymd_and_hms(2026, 5, 30, 13, 0, 0).unwrap();
    let allow = win(SyncWindowKind::Allow, "0 10 * * *", "1h", false, &[], &[], &[]);
    assert!(!can_sync(std::slice::from_ref(&allow), now, false));
    // but a manual-enabled inactive allow permits manual sync
    let allow_m = win(SyncWindowKind::Allow, "0 10 * * *", "1h", true, &[], &[], &[]);
    assert!(can_sync(std::slice::from_ref(&allow_m), now, true));
    assert!(!can_sync(std::slice::from_ref(&allow_m), now, false));
}

// ─── Matches (selector glob) ────────────────────────────────────────────────

#[test]
fn matching_windows_filters_by_app_namespace_cluster() {
    let windows = vec![
        win(SyncWindowKind::Deny, "* * * * *", "1h", false, &["prod-*"], &[], &[]),
        win(SyncWindowKind::Allow, "* * * * *", "1h", false, &[], &["staging"], &[]),
        win(SyncWindowKind::Deny, "* * * * *", "1h", false, &[], &[], &["https://prod.example.com"]),
    ];
    // prod-web app matches window 0 only
    let m = matching_windows(&windows, "prod-web", "default", "https://dev.example.com");
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].kind, SyncWindowKind::Deny);

    // staging namespace matches window 1
    let m2 = matching_windows(&windows, "other", "staging", "https://dev.example.com");
    assert_eq!(m2.len(), 1);
    assert_eq!(m2[0].kind, SyncWindowKind::Allow);

    // prod cluster matches window 2
    let m3 = matching_windows(&windows, "other", "default", "https://prod.example.com");
    assert_eq!(m3.len(), 1);
}

#[test]
fn matching_windows_empty_selectors_apply_to_all() {
    let windows = vec![win(
        SyncWindowKind::Deny,
        "* * * * *",
        "1h",
        false,
        &[],
        &[],
        &[],
    )];
    let m = matching_windows(&windows, "anything", "anywhere", "any-cluster");
    assert_eq!(m.len(), 1);
}
