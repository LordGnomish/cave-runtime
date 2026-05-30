// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Sync-window evaluation — faithful in-crate port of
//! argoproj/argo-cd v3.4.2 `util/argo/sync_windows.go`
//! (source_sha 0dc6b1b57dd5bb925d5b03c3d09419ab9fb4225e).
//!
//! ArgoCD `AppProject.spec.syncWindows[]` gate when an Application may sync.
//! Each window has a `kind` (allow / deny), a 5-field cron `schedule`, a
//! `duration`, optional application/namespace/cluster selectors, and a
//! `manualSync` flag. This module ports the pure decision logic:
//!
//!   * `SyncWindow.Active()`         → [`window_is_active`]
//!   * `SyncWindows.matchingWindows` → [`matching_windows`]
//!   * `SyncWindows.InactiveAllows`  → [`inactive_allows`]
//!   * `SyncWindows.CanSync(manual)` → [`can_sync`]
//!
//! Upstream parses the cron schedule with `github.com/robfig/cron/v3`; we port a
//! small standard 5-field cron matcher in-crate (no new workspace dependency)
//! and replicate `schedule.Next`/`Prev` semantics by scanning minute-by-minute
//! back over the window `duration`. All logic is pure: no persistence, network,
//! or subprocess.

use crate::rbac::{SyncWindow, SyncWindowKind};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};

/// The application identity a window selector is matched against.
///
/// Mirrors the fields ArgoCD's `SyncWindows.Matches` reads off an
/// `Application`: its name, destination namespace and destination cluster.
#[derive(Debug, Clone)]
pub struct WindowAppContext {
    pub app_name: String,
    pub namespace: String,
    pub cluster: String,
}

/// Parse a Go-style duration string (`"1h"`, `"30m"`, `"1h30m"`, `"45s"`).
///
/// Port of the `time.ParseDuration` subset ArgoCD passes from
/// `SyncWindow.Duration`. Returns `None` on malformed input (upstream treats a
/// parse error as "window never opens").
pub fn parse_go_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut saw_unit = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
            continue;
        }
        if num.is_empty() {
            return None;
        }
        let value: i64 = num.parse().ok()?;
        let unit = match ch {
            'h' => Duration::hours(value),
            'm' => Duration::minutes(value),
            's' => Duration::seconds(value),
            _ => return None,
        };
        total = total + unit;
        num.clear();
        saw_unit = true;
    }
    // A trailing bare number (no unit) is invalid in Go's ParseDuration.
    if !num.is_empty() || !saw_unit {
        return None;
    }
    Some(total)
}

/// One field of a 5-field cron expression.
#[derive(Debug, Clone)]
enum CronField {
    Any,
    /// Explicit set of allowed values for this position.
    Values(Vec<u32>),
}

impl CronField {
    fn matches(&self, v: u32) -> bool {
        match self {
            CronField::Any => true,
            CronField::Values(set) => set.contains(&v),
        }
    }

    /// Parse a single cron field given the inclusive `[min, max]` legal range.
    /// Supports `*`, `*/step`, comma lists, `a-b` ranges, and `a-b/step`.
    fn parse(spec: &str, min: u32, max: u32) -> Option<CronField> {
        if spec == "*" {
            return Some(CronField::Any);
        }
        let mut out: Vec<u32> = Vec::new();
        for part in spec.split(',') {
            let (range_part, step) = match part.split_once('/') {
                Some((r, s)) => (r, s.parse::<u32>().ok().filter(|&n| n != 0)?),
                None => (part, 1),
            };
            let (lo, hi) = if range_part == "*" {
                (min, max)
            } else if let Some((a, b)) = range_part.split_once('-') {
                (a.parse().ok()?, b.parse().ok()?)
            } else {
                let v: u32 = range_part.parse().ok()?;
                (v, v)
            };
            if lo < min || hi > max || lo > hi {
                return None;
            }
            let mut v = lo;
            while v <= hi {
                out.push(v);
                v += step;
            }
        }
        if out.is_empty() {
            return None;
        }
        Some(CronField::Values(out))
    }
}

/// A parsed standard 5-field cron schedule:
/// `minute hour day-of-month month day-of-week`.
///
/// Equivalent to the standard parser ArgoCD obtains from `robfig/cron`'s
/// `cron.ParseStandard`. Day-of-week 0 and 7 both mean Sunday. Day-of-month and
/// day-of-week combine with OR semantics when both are restricted (the standard
/// Vixie-cron rule robfig/cron follows).
#[derive(Debug, Clone)]
pub struct CronSchedule {
    minute: CronField,
    hour: CronField,
    dom: CronField,
    month: CronField,
    dow: CronField,
    dom_restricted: bool,
    dow_restricted: bool,
}

impl CronSchedule {
    /// Parse a 5-field cron string. Returns `None` if it is not exactly five
    /// whitespace-separated fields or any field is malformed.
    pub fn parse(spec: &str) -> Option<CronSchedule> {
        let fields: Vec<&str> = spec.split_whitespace().collect();
        if fields.len() != 5 {
            return None;
        }
        // Day-of-week: accept 0-7, then fold 7 → 0 (Sunday).
        let dow_raw = CronField::parse(fields[4], 0, 7)?;
        let dow = match dow_raw {
            CronField::Any => CronField::Any,
            CronField::Values(mut v) => {
                for x in v.iter_mut() {
                    if *x == 7 {
                        *x = 0;
                    }
                }
                v.sort_unstable();
                v.dedup();
                CronField::Values(v)
            }
        };
        Some(CronSchedule {
            minute: CronField::parse(fields[0], 0, 59)?,
            hour: CronField::parse(fields[1], 0, 23)?,
            dom: CronField::parse(fields[2], 1, 31)?,
            month: CronField::parse(fields[3], 1, 12)?,
            dow,
            dom_restricted: fields[2] != "*",
            dow_restricted: fields[4] != "*",
        })
    }

    /// Does this schedule fire at the given (minute-resolution) instant?
    pub fn fires_at(&self, t: DateTime<Utc>) -> bool {
        let minute_ok = self.minute.matches(t.minute());
        let hour_ok = self.hour.matches(t.hour());
        let month_ok = self.month.matches(t.month());
        // chrono weekday: Mon=0..Sun=6; cron: Sun=0..Sat=6.
        let cron_dow = (t.weekday().num_days_from_sunday()) as u32;
        let dom_ok = self.dom.matches(t.day());
        let dow_ok = self.dow.matches(cron_dow);
        // Vixie-cron day matching: if both DOM and DOW are restricted, OR them;
        // otherwise AND (where an unrestricted field is always true).
        let day_ok = if self.dom_restricted && self.dow_restricted {
            dom_ok || dow_ok
        } else {
            dom_ok && dow_ok
        };
        minute_ok && hour_ok && day_ok && month_ok
    }
}

/// Port of `SyncWindow.Active()`.
///
/// The window opens at every cron firing and stays open for `duration`. We
/// determine activity by scanning back minute-by-minute from `now` across the
/// window duration: if any of those minutes is a cron firing, the window is
/// active at `now`. A malformed schedule or duration yields `false`
/// (matching upstream, which logs the parse error and treats the window as
/// inactive).
pub fn window_is_active(window: &SyncWindow, now: DateTime<Utc>) -> bool {
    let schedule = match CronSchedule::parse(&window.schedule) {
        Some(s) => s,
        None => return false,
    };
    let duration = match parse_go_duration(&window.duration) {
        Some(d) if d > Duration::zero() => d,
        _ => return false,
    };
    // Normalize to minute resolution (cron fires on minute boundaries).
    let now_minute = Utc
        .with_ymd_and_hms(now.year(), now.month(), now.day(), now.hour(), now.minute(), 0)
        .single()
        .unwrap_or(now);
    let span_minutes = duration.num_minutes();
    let mut offset = 0;
    while offset <= span_minutes {
        let candidate = now_minute - Duration::minutes(offset);
        if schedule.fires_at(candidate) {
            // Open from `candidate` for `duration`; active iff now < candidate+duration.
            if now < candidate + duration {
                return true;
            }
        }
        offset += 1;
    }
    false
}

/// Does a glob/exact selector list match the given value?
///
/// Empty list = "matches all" (upstream treats an empty selector as a wildcard).
fn selector_matches(patterns: &[String], value: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| crate::rbac::glob_match(p, value))
}

/// Port of `SyncWindows.matchingWindows(app)` — keep windows whose
/// application/namespace/cluster selectors all match the app context.
pub fn matching_windows<'a>(
    windows: &'a [SyncWindow],
    ctx: &WindowAppContext,
) -> Vec<&'a SyncWindow> {
    windows
        .iter()
        .filter(|w| {
            selector_matches(&w.applications, &ctx.app_name)
                && selector_matches(&w.namespaces, &ctx.namespace)
                && selector_matches(&w.clusters, &ctx.cluster)
        })
        .collect()
}

/// Port of `SyncWindows.InactiveAllows()`.
///
/// True when at least one *allow* window exists but none of the allow windows
/// is currently active. In that state ArgoCD defaults to deny (sync is only
/// permitted inside an active allow window).
pub fn inactive_allows(windows: &[SyncWindow], now: DateTime<Utc>) -> bool {
    let mut has_allow = false;
    let mut has_active_allow = false;
    for w in windows {
        if w.kind == SyncWindowKind::Allow {
            has_allow = true;
            if window_is_active(w, now) {
                has_active_allow = true;
            }
        }
    }
    has_allow && !has_active_allow
}

/// Port of `SyncWindows.CanSync(isManual bool)`.
///
/// Decision order (matching upstream):
///   1. No windows → allowed.
///   2. Any active *deny* window → denied, unless this is a manual sync and that
///      deny window permits manual sync (`manualSync`).
///   3. If allow windows exist (`inactiveAllows`) and none is active → denied,
///      unless this is a manual sync and an allow window permits manual sync.
///   4. Otherwise → allowed.
pub fn can_sync(windows: &[SyncWindow], is_manual: bool, now: DateTime<Utc>) -> bool {
    if windows.is_empty() {
        return true;
    }

    // Active deny windows block everything (manual override per-window).
    let mut active_deny_present = false;
    let mut active_deny_manual_ok = false;
    for w in windows {
        if w.kind == SyncWindowKind::Deny && window_is_active(w, now) {
            active_deny_present = true;
            if w.manual_sync {
                active_deny_manual_ok = true;
            }
        }
    }
    if active_deny_present {
        return is_manual && active_deny_manual_ok;
    }

    // No active deny. If allow windows gate us out, check manual override.
    if inactive_allows(windows, now) {
        if is_manual {
            // A manual sync is permitted if any allow window allows manual sync.
            return windows
                .iter()
                .any(|w| w.kind == SyncWindowKind::Allow && w.manual_sync);
        }
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow(schedule: &str, duration: &str) -> SyncWindow {
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

    #[test]
    fn parse_duration_compound() {
        assert_eq!(
            parse_go_duration("1h30m"),
            Some(Duration::minutes(90))
        );
        assert_eq!(parse_go_duration("45s"), Some(Duration::seconds(45)));
        assert_eq!(parse_go_duration("bad"), None);
        assert_eq!(parse_go_duration("10"), None);
    }

    #[test]
    fn cron_parse_and_fire() {
        let s = CronSchedule::parse("30 9 * * 1-5").unwrap();
        // Friday 2026-05-29 09:30 UTC fires.
        let t = Utc.with_ymd_and_hms(2026, 5, 29, 9, 30, 0).unwrap();
        assert!(s.fires_at(t));
        // Saturday does not.
        let sat = Utc.with_ymd_and_hms(2026, 5, 30, 9, 30, 0).unwrap();
        assert!(!s.fires_at(sat));
    }

    #[test]
    fn step_field_parses() {
        let s = CronSchedule::parse("*/15 * * * *").unwrap();
        let t = Utc.with_ymd_and_hms(2026, 5, 30, 0, 45, 0).unwrap();
        assert!(s.fires_at(t));
        let t2 = Utc.with_ymd_and_hms(2026, 5, 30, 0, 46, 0).unwrap();
        assert!(!s.fires_at(t2));
    }

    #[test]
    fn active_then_inactive() {
        let w = allow("0 10 * * *", "1h");
        assert!(window_is_active(
            &w,
            Utc.with_ymd_and_hms(2026, 5, 30, 10, 59, 0).unwrap()
        ));
        assert!(!window_is_active(
            &w,
            Utc.with_ymd_and_hms(2026, 5, 30, 11, 1, 0).unwrap()
        ));
    }

    #[test]
    fn malformed_window_is_inactive() {
        let mut w = allow("not a cron", "1h");
        assert!(!window_is_active(
            &w,
            Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap()
        ));
        w.schedule = "0 10 * * *".to_string();
        w.duration = "garbage".to_string();
        assert!(!window_is_active(
            &w,
            Utc.with_ymd_and_hms(2026, 5, 30, 10, 30, 0).unwrap()
        ));
    }
}
