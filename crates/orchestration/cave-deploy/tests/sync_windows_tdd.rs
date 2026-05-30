// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD RED cycle 2026-05-30 — converts the `sync-windows-cron` skip into a
// real in-crate mapped subsystem.
//
// Ports argoproj/argo-cd v3.4.2 `util/argo/sync_windows.go`:
//   * SyncWindow.Active()        → window_is_active (cron schedule + duration)
//   * SyncWindows.Matches(app)   → matching_windows (applications/namespaces/clusters glob)
//   * SyncWindows.CanSync(manual)→ can_sync (deny-precedence + allow-gating + manualSync)
//   * SyncWindows.InactiveAllows → inactive_allows
//
// All pure in-crate time/cron logic — no persistence, no network, no subprocess.

use cave_deploy::rbac::{SyncWindow, SyncWindowKind};
use cave_deploy::sync_windows::{
    can_sync, inactive_allows, matching_windows, window_is_active, WindowAppContext,
};
use chrono::{TimeZone, Utc};

fn allow_window(schedule: &str, duration: &str) -> SyncWindow {
    SyncWindow {
        kind: SyncWindowKind::Allow,
        schedule: schedule.to_string(),
        duration: duration.to_string(),
        applications: vec![],
        namespaces: vec![],
        clusters: vec![],
        manual_sync: false,
        time_zone: None,
    }
}

fn deny_window(schedule: &str, duration: &str) -> SyncWindow {
    SyncWindow {
        kind: SyncWindowKind::Deny,
        ..allow_window(schedule, duration)
    }
}

#[test]
fn cron_window_active_inside_duration() {
    // Window opens every day at 10:00 for 1h. 10:30 is inside.
    let w = allow_window("0 10 * * *", "1h");
    let now = Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap();
    assert!(window_is_active(&w, now));
}

#[test]
fn cron_window_inactive_after_duration() {
    // Window opens at 10:00 for 1h. 11:30 is outside.
    let w = allow_window("0 10 * * *", "1h");
    let now = Utc.with_ymd_and_hms(2026, 5, 30, 11, 30, 0).unwrap();
    assert!(!window_is_active(&w, now));
}

#[test]
fn cron_window_active_at_open_minute() {
    let w = allow_window("0 10 * * *", "30m");
    let now = Utc.with_ymd_and_hms(2026, 5, 30, 10, 0, 0).unwrap();
    assert!(window_is_active(&w, now));
}

#[test]
fn matching_windows_filters_by_application_glob() {
    let mut w = allow_window("* * * * *", "1h");
    w.applications = vec!["prod-*".to_string()];
    let windows = vec![w];
    let ctx = WindowAppContext {
        app_name: "prod-web".to_string(),
        namespace: "default".to_string(),
        cluster: "https://kubernetes.default.svc".to_string(),
    };
    assert_eq!(matching_windows(&windows, &ctx).len(), 1);

    let ctx2 = WindowAppContext {
        app_name: "dev-web".to_string(),
        ..ctx
    };
    assert_eq!(matching_windows(&windows, &ctx2).len(), 0);
}

#[test]
fn active_deny_blocks_automated_sync_but_allows_manual_when_enabled() {
    // A deny window that is currently active blocks automated sync.
    let mut deny = deny_window("0 10 * * *", "2h");
    deny.manual_sync = true;
    let windows = vec![deny];
    let now = Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap();
    // automated → blocked
    assert!(!can_sync(&windows, false, now));
    // manual + manualSync=true → allowed
    assert!(can_sync(&windows, true, now));
}

#[test]
fn allow_window_gates_sync_to_its_active_period() {
    // Only an allow window exists. Sync permitted only while it is active.
    let windows = vec![allow_window("0 10 * * *", "1h")];
    let inside = Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap();
    let outside = Utc.with_ymd_and_hms(2026, 5, 30, 14, 0, 0).unwrap();
    assert!(can_sync(&windows, false, inside));
    assert!(!can_sync(&windows, false, outside));
}

#[test]
fn no_windows_means_always_can_sync() {
    let windows: Vec<SyncWindow> = vec![];
    let now = Utc.with_ymd_and_hms(2026, 5, 30, 3, 0, 0).unwrap();
    assert!(can_sync(&windows, false, now));
}

#[test]
fn inactive_allow_window_present_means_default_deny() {
    // When allow windows exist but none active, inactive_allows is true,
    // meaning syncs are denied outside the allow periods.
    let windows = vec![allow_window("0 10 * * *", "1h")];
    let outside = Utc.with_ymd_and_hms(2026, 5, 30, 14, 0, 0).unwrap();
    assert!(inactive_allows(&windows, outside));
}
