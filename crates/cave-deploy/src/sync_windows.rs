// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sync windows — pure-Rust port of ArgoCD's maintenance-window gate.
//!
//! Upstream: `pkg/apis/application/v1alpha1/types.go`
//! `SyncWindows.{Active,CanSync,Matches,InactiveAllows}` plus the
//! robfig/cron schedule evaluation that `controller/state.go` uses to decide
//! whether an auto-sync (or manual sync) may proceed inside an allow/deny
//! window.
//!
//! The [`SyncWindow`] CRD shape lives in [`crate::rbac`]; this module owns the
//! evaluation algebra:
//!
//!   * [`CronSchedule`]    — 5-field cron parser + [`CronSchedule::next`]
//!   * [`parse_duration`]  — Go `time.ParseDuration` subset
//!   * [`window_active`]   — is a window active at instant `now`
//!   * [`can_sync`]        — allow/deny precedence (auto vs manual)
//!   * [`matching_windows`]— selector glob (applications / namespaces / clusters)
//!
//! No subprocess, no external cron crate — dependency-light by design.

use crate::error::DeployError;
use crate::rbac::{SyncWindow, SyncWindowKind};
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};

// ─── Cron schedule ──────────────────────────────────────────────────────────

/// A parsed 5-field cron expression (`minute hour day-of-month month
/// day-of-week`).  Each field expands to an explicit allowed-value set, so
/// matching is a membership test.
#[derive(Debug, Clone)]
pub struct CronSchedule {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    doms: Vec<u32>,
    months: Vec<u32>,
    dows: Vec<u32>,
    /// True when the day-of-month field was restricted (not `*`/`?`).
    dom_restricted: bool,
    /// True when the day-of-week field was restricted (not `*`/`?`).
    dow_restricted: bool,
}

impl CronSchedule {
    /// Parse a standard 5-field cron expression.  Supports `*`, `?` (= `*`),
    /// lists (`a,b`), ranges (`a-b`), and steps (`*/n`, `a-b/n`).
    pub fn parse(expr: &str) -> Result<CronSchedule, DeployError> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(DeployError::Invalid(format!(
                "cron expression must have 5 fields, got {} in '{}'",
                fields.len(),
                expr
            )));
        }
        let minutes = expand_field(fields[0], 0, 59)?;
        let hours = expand_field(fields[1], 0, 23)?;
        let doms = expand_field(fields[2], 1, 31)?;
        let months = expand_field(fields[3], 1, 12)?;
        // day-of-week 0-6 (0 = Sunday); 7 is normalised to 0.
        let dows_raw = expand_field(fields[4], 0, 7)?;
        let dows: Vec<u32> = dows_raw.into_iter().map(|d| if d == 7 { 0 } else { d }).collect();
        let is_wild = |f: &str| f == "*" || f == "?";
        Ok(CronSchedule {
            minutes,
            hours,
            doms,
            months,
            dows,
            dom_restricted: !is_wild(fields[2]),
            dow_restricted: !is_wild(fields[4]),
        })
    }

    /// Does instant `t` (minute resolution) match this schedule?
    pub fn matches_time(&self, t: DateTime<Utc>) -> bool {
        if !self.minutes.contains(&t.minute()) {
            return false;
        }
        if !self.hours.contains(&t.hour()) {
            return false;
        }
        if !self.months.contains(&t.month()) {
            return false;
        }
        // weekday(): Mon=0..Sun=6 in chrono; convert to cron Sun=0..Sat=6.
        let cron_dow = (t.weekday().num_days_from_sunday()) % 7;
        let dom_ok = self.doms.contains(&t.day());
        let dow_ok = self.dows.contains(&cron_dow);
        // Standard cron: if BOTH dom and dow are restricted, match on EITHER.
        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => dom_ok || dow_ok,
            (true, false) => dom_ok,
            (false, true) => dow_ok,
            (false, false) => true,
        }
    }

    /// The next activation strictly after `after` (minute resolution), or
    /// `None` if none found within a ~4-year search horizon.
    pub fn next(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        // start at the next whole minute strictly after `after`.
        let mut t = (after + Duration::minutes(1))
            .with_second(0)?
            .with_nanosecond(0)?;
        // bound the search: 4 years of minutes.
        for _ in 0..(4 * 366 * 24 * 60) {
            if self.matches_time(t) {
                return Some(t);
            }
            t += Duration::minutes(1);
        }
        None
    }
}

fn expand_field(field: &str, min: u32, max: u32) -> Result<Vec<u32>, DeployError> {
    let mut out = Vec::new();
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // step: "<range>/<n>"
        let (range_spec, step) = match part.split_once('/') {
            Some((r, s)) => {
                let n: u32 = s
                    .parse()
                    .map_err(|_| DeployError::Invalid(format!("bad cron step '{}'", s)))?;
                if n == 0 {
                    return Err(DeployError::Invalid("cron step cannot be 0".into()));
                }
                (r, n)
            }
            None => (part, 1),
        };
        let (lo, hi) = if range_spec == "*" || range_spec == "?" {
            (min, max)
        } else if let Some((a, b)) = range_spec.split_once('-') {
            (
                a.parse()
                    .map_err(|_| DeployError::Invalid(format!("bad cron range '{}'", range_spec)))?,
                b.parse()
                    .map_err(|_| DeployError::Invalid(format!("bad cron range '{}'", range_spec)))?,
            )
        } else {
            let v: u32 = range_spec
                .parse()
                .map_err(|_| DeployError::Invalid(format!("bad cron value '{}'", range_spec)))?;
            // a single value with a step (e.g. "5/15") ranges value..max
            if step > 1 {
                (v, max)
            } else {
                (v, v)
            }
        };
        if lo < min || hi > max || lo > hi {
            return Err(DeployError::Invalid(format!(
                "cron field '{}' out of range [{}, {}]",
                part, min, max
            )));
        }
        let mut v = lo;
        while v <= hi {
            if !out.contains(&v) {
                out.push(v);
            }
            v += step;
        }
    }
    if out.is_empty() {
        return Err(DeployError::Invalid(format!("empty cron field '{}'", field)));
    }
    out.sort_unstable();
    Ok(out)
}

// ─── Duration parsing (Go time.ParseDuration subset) ────────────────────────

/// Parse a Go-style duration string (`1h`, `30m`, `2h30m`, `45s`).  Returns
/// `None` on malformed input.
pub fn parse_duration(s: &str) -> Option<Duration> {
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
        } else {
            if num.is_empty() {
                return None;
            }
            let n: i64 = num.parse().ok()?;
            let unit = match ch {
                'h' => Duration::hours(n),
                'm' => Duration::minutes(n),
                's' => Duration::seconds(n),
                _ => return None,
            };
            total += unit;
            num.clear();
            saw_unit = true;
        }
    }
    // trailing digits with no unit are invalid
    if !num.is_empty() || !saw_unit {
        return None;
    }
    Some(total)
}

// ─── Window activeness ──────────────────────────────────────────────────────

/// Is `window` active at instant `now`?
///
/// Mirrors ArgoCD `(w *SyncWindow) active`: with `start =
/// schedule.Next(now - duration)`, the window is active when
/// `start < now < start + duration`.
pub fn window_active(window: &SyncWindow, now: DateTime<Utc>) -> bool {
    let Ok(schedule) = CronSchedule::parse(&window.schedule) else {
        return false;
    };
    let Some(duration) = parse_duration(&window.duration) else {
        return false;
    };
    let Some(start) = schedule.next(now - duration) else {
        return false;
    };
    let end = start + duration;
    now > start && now < end
}

// ─── CanSync precedence ─────────────────────────────────────────────────────

/// Decide whether a sync may proceed at `now`, honouring allow/deny windows.
///
/// Faithful to ArgoCD `(w *SyncWindows) CanSync(isManual bool)`:
///   1. No windows → allowed.
///   2. Any active **deny** → blocked, unless `is_manual` and *every* active
///      deny window enables manual sync.
///   3. Any active **allow** → allowed.
///   4. Otherwise, if inactive allow windows exist → blocked, unless
///      `is_manual` and at least one inactive allow enables manual sync.
///   5. Else → allowed.
pub fn can_sync(windows: &[SyncWindow], now: DateTime<Utc>, is_manual: bool) -> bool {
    if windows.is_empty() {
        return true;
    }
    let active: Vec<&SyncWindow> = windows.iter().filter(|w| window_active(w, now)).collect();

    // active deny windows
    let active_denies: Vec<&&SyncWindow> = active
        .iter()
        .filter(|w| w.kind == SyncWindowKind::Deny)
        .collect();
    if !active_denies.is_empty() {
        // manual allowed only if ALL active deny windows enable manual sync.
        let manual_enabled = active_denies.iter().all(|w| w.manual_sync);
        return is_manual && manual_enabled;
    }

    // active allow windows
    if active.iter().any(|w| w.kind == SyncWindowKind::Allow) {
        return true;
    }

    // inactive allow windows
    let inactive_allows: Vec<&SyncWindow> = windows
        .iter()
        .filter(|w| w.kind == SyncWindowKind::Allow && !window_active(w, now))
        .collect();
    if !inactive_allows.is_empty() {
        let manual_enabled = inactive_allows.iter().any(|w| w.manual_sync);
        return is_manual && manual_enabled;
    }

    true
}

// ─── Selector matching ──────────────────────────────────────────────────────

/// The subset of `windows` that apply to the given application coordinates.
///
/// Mirrors ArgoCD `(w *SyncWindows) Matches(app)`: a window applies if any of
/// its `applications` / `namespaces` / `clusters` selectors glob-matches the
/// app name / destination namespace / destination cluster.  A window with no
/// selectors at all applies to everything.
pub fn matching_windows<'a>(
    windows: &'a [SyncWindow],
    app_name: &str,
    namespace: &str,
    cluster: &str,
) -> Vec<&'a SyncWindow> {
    windows
        .iter()
        .filter(|w| {
            if w.applications.is_empty() && w.namespaces.is_empty() && w.clusters.is_empty() {
                return true;
            }
            w.applications.iter().any(|p| glob_match(p, app_name))
                || w.namespaces.iter().any(|p| glob_match(p, namespace))
                || w.clusters.iter().any(|p| glob_match(p, cluster))
        })
        .collect()
}

/// Minimal glob matcher supporting `*` (any run, including empty) and `?`
/// (single char) — matches ArgoCD's use of `glob.Match` for window selectors.
fn glob_match(pattern: &str, value: &str) -> bool {
    fn helper(p: &[u8], v: &[u8]) -> bool {
        match p.first() {
            None => v.is_empty(),
            Some(b'*') => helper(&p[1..], v) || (!v.is_empty() && helper(p, &v[1..])),
            Some(b'?') => !v.is_empty() && helper(&p[1..], &v[1..]),
            Some(&c) => !v.is_empty() && v[0] == c && helper(&p[1..], &v[1..]),
        }
    }
    helper(pattern.as_bytes(), value.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn glob_basics() {
        assert!(glob_match("prod-*", "prod-web"));
        assert!(!glob_match("prod-*", "dev-web"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("app-?", "app-1"));
        assert!(!glob_match("app-?", "app-12"));
    }

    #[test]
    fn dom_or_dow_when_both_restricted() {
        // 15th of the month OR Monday
        let s = CronSchedule::parse("0 0 15 * 1").unwrap();
        // 2026-05-15 is a Friday → matches via day-of-month
        let dom = Utc
            .with_ymd_and_hms(2026, 5, 15, 0, 0, 0)
            .single()
            .unwrap();
        assert!(s.matches_time(dom));
        // 2026-06-01 is a Monday → matches via day-of-week
        let dow = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).single().unwrap();
        assert!(s.matches_time(dow));
    }
}
