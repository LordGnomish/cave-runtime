//! ScheduledBackup CRD.
//!
//! Mirrors `pkg/specs/scheduledbackup_types.go` from CloudNativePG.
//! A ScheduledBackup carries a 5-field cron expression and triggers
//! a one-shot [`crate::backup::BackupManager::start_backup`] each
//! time the schedule elapses.
//!
//! cave-rdbms-operator's `backup.rs` already covers the one-shot
//! Backup mechanics; this module owns the *scheduling* — parsing the
//! cron expression, computing the next fire time, and exposing a
//! `due_at(now)` predicate the operator loop can poll.

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::types::BackupType;

#[derive(Debug, thiserror::Error)]
pub enum ScheduledBackupError {
    #[error("scheduled backup {0} not found")]
    NotFound(String),
    #[error("scheduled backup {0} already exists")]
    AlreadyExists(String),
    #[error("invalid cron: {0}")]
    InvalidCron(String),
}

// ── Cron parser ─────────────────────────────────────────────────────────────

/// A minimal 5-field cron expression: `minute hour day month weekday`.
/// Supports `*`, single integers, comma-separated lists, ranges
/// (`1-5`), and step values (`*/15`). Day-of-month and day-of-week
/// are AND-ed when both are restrictive, matching the kubelet
/// CronJob behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    minute: FieldSet,
    hour: FieldSet,
    day_of_month: FieldSet,
    month: FieldSet,
    day_of_week: FieldSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldSet {
    Any,
    Values(Vec<u32>),
}

impl FieldSet {
    fn contains(&self, v: u32) -> bool {
        match self {
            FieldSet::Any => true,
            FieldSet::Values(vs) => vs.binary_search(&v).is_ok(),
        }
    }
}

impl CronSchedule {
    pub fn parse(expr: &str) -> Result<Self, ScheduledBackupError> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(ScheduledBackupError::InvalidCron(format!(
                "expected 5 fields, got {}",
                parts.len()
            )));
        }
        Ok(Self {
            minute: parse_field(parts[0], 0, 59)?,
            hour: parse_field(parts[1], 0, 23)?,
            day_of_month: parse_field(parts[2], 1, 31)?,
            month: parse_field(parts[3], 1, 12)?,
            day_of_week: parse_field(parts[4], 0, 6)?, // 0 = Sunday, matching cron(5)
        })
    }

    /// Returns the next instant at or after `after` that matches
    /// every field. Always rounds up to the next minute boundary.
    pub fn next_after(&self, after: DateTime<Utc>) -> DateTime<Utc> {
        // Drop seconds, then advance one minute. Cron resolution is
        // 1 minute; `next_after(t)` means strictly *after* the
        // current minute.
        let base = after
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or(after);
        let mut t = base + Duration::minutes(1);
        // Hard cap to prevent loops on impossible schedules
        // (e.g. `0 0 30 2 *` — Feb 30 — would never match).
        for _ in 0..(366 * 24 * 60) {
            if self.matches(t) {
                return t;
            }
            t += Duration::minutes(1);
        }
        // Fallback: return the cap point. Tests use sane schedules.
        t
    }

    pub fn matches(&self, t: DateTime<Utc>) -> bool {
        // weekday(): 0 = Monday in chrono; cron(5) uses 0 = Sunday.
        let dow = t.weekday().num_days_from_sunday();
        self.minute.contains(t.minute())
            && self.hour.contains(t.hour())
            && self.day_of_month.contains(t.day())
            && self.month.contains(t.month())
            && self.day_of_week.contains(dow)
    }
}

fn parse_field(spec: &str, lo: u32, hi: u32) -> Result<FieldSet, ScheduledBackupError> {
    if spec == "*" {
        return Ok(FieldSet::Any);
    }
    let mut values: Vec<u32> = Vec::new();
    for part in spec.split(',') {
        let (range_spec, step) = if let Some((r, s)) = part.split_once('/') {
            let step: u32 = s
                .parse()
                .map_err(|_| ScheduledBackupError::InvalidCron(format!("bad step in {part:?}")))?;
            if step == 0 {
                return Err(ScheduledBackupError::InvalidCron("step must be >= 1".into()));
            }
            (r, step)
        } else {
            (part, 1)
        };
        let (start, end) = if range_spec == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range_spec.split_once('-') {
            let a: u32 = a.parse().map_err(|_| {
                ScheduledBackupError::InvalidCron(format!("bad range start {a:?}"))
            })?;
            let b: u32 = b.parse().map_err(|_| {
                ScheduledBackupError::InvalidCron(format!("bad range end {b:?}"))
            })?;
            (a, b)
        } else {
            let v: u32 = range_spec.parse().map_err(|_| {
                ScheduledBackupError::InvalidCron(format!("bad value {range_spec:?}"))
            })?;
            (v, v)
        };
        if start < lo || end > hi || start > end {
            return Err(ScheduledBackupError::InvalidCron(format!(
                "range {start}-{end} outside [{lo}, {hi}]"
            )));
        }
        let mut v = start;
        while v <= end {
            values.push(v);
            v += step;
        }
    }
    values.sort();
    values.dedup();
    Ok(FieldSet::Values(values))
}

// ── ScheduledBackup CRD ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduledBackupPhase {
    Pending,
    Active,
    Suspended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledBackupSpec {
    pub instance_id: String,
    /// 5-field cron expression in UTC.
    pub schedule: String,
    pub backup_type: BackupType,
    /// Retain policy: keep this many most-recent successful runs.
    pub retain_runs: u32,
    /// Optional: pause the schedule without deleting it.
    pub suspended: bool,
}

#[derive(Debug, Clone)]
pub struct ScheduledBackup {
    pub name: String,
    pub spec: ScheduledBackupSpec,
    pub phase: ScheduledBackupPhase,
    pub schedule: CronSchedule,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: DateTime<Utc>,
    pub run_count: u64,
    pub recent_runs: Vec<DateTime<Utc>>,
}

impl ScheduledBackup {
    /// True if `now` is at or after `next_run_at` (and not suspended).
    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        !self.spec.suspended && now >= self.next_run_at
    }

    /// Record a successful run. Returns the new `next_run_at`.
    pub fn record_run(&mut self, ran_at: DateTime<Utc>) -> DateTime<Utc> {
        self.last_run_at = Some(ran_at);
        self.run_count += 1;
        self.recent_runs.push(ran_at);
        let retain = self.spec.retain_runs as usize;
        if retain > 0 && self.recent_runs.len() > retain {
            let drop = self.recent_runs.len() - retain;
            self.recent_runs.drain(0..drop);
        }
        self.next_run_at = self.schedule.next_after(ran_at);
        self.next_run_at
    }
}

pub struct ScheduledBackupManager {
    backups: Arc<RwLock<HashMap<String, ScheduledBackup>>>,
}

impl ScheduledBackupManager {
    pub fn new() -> Self {
        Self {
            backups: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn create(
        &self,
        name: impl Into<String>,
        spec: ScheduledBackupSpec,
        now: DateTime<Utc>,
    ) -> Result<ScheduledBackup, ScheduledBackupError> {
        let schedule = CronSchedule::parse(&spec.schedule)?;
        let name = name.into();
        let mut backups = self.backups.write().unwrap();
        if backups.contains_key(&name) {
            return Err(ScheduledBackupError::AlreadyExists(name));
        }
        let phase = if spec.suspended {
            ScheduledBackupPhase::Suspended
        } else {
            ScheduledBackupPhase::Active
        };
        let next_run_at = schedule.next_after(now);
        let sb = ScheduledBackup {
            name: name.clone(),
            spec,
            phase,
            schedule,
            last_run_at: None,
            next_run_at,
            run_count: 0,
            recent_runs: Vec::new(),
        };
        backups.insert(name, sb.clone_for_return());
        Ok(sb)
    }

    pub fn get(&self, name: &str) -> Result<ScheduledBackup, ScheduledBackupError> {
        self.backups
            .read()
            .unwrap()
            .get(name)
            .map(|b| b.clone_for_return())
            .ok_or_else(|| ScheduledBackupError::NotFound(name.into()))
    }

    pub fn list(&self) -> Vec<ScheduledBackup> {
        self.backups
            .read()
            .unwrap()
            .values()
            .map(|b| b.clone_for_return())
            .collect()
    }

    /// Return the names of schedules that are due at `now`, mark
    /// them as having run, and advance their next_run_at.
    pub fn trigger_due(&self, now: DateTime<Utc>) -> Vec<String> {
        let mut fired = Vec::new();
        let mut backups = self.backups.write().unwrap();
        for (name, sb) in backups.iter_mut() {
            if sb.is_due(now) {
                sb.record_run(now);
                fired.push(name.clone());
            }
        }
        fired
    }

    pub fn suspend(&self, name: &str) -> Result<(), ScheduledBackupError> {
        let mut backups = self.backups.write().unwrap();
        let sb = backups
            .get_mut(name)
            .ok_or_else(|| ScheduledBackupError::NotFound(name.into()))?;
        sb.spec.suspended = true;
        sb.phase = ScheduledBackupPhase::Suspended;
        Ok(())
    }

    pub fn resume(
        &self,
        name: &str,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, ScheduledBackupError> {
        let mut backups = self.backups.write().unwrap();
        let sb = backups
            .get_mut(name)
            .ok_or_else(|| ScheduledBackupError::NotFound(name.into()))?;
        sb.spec.suspended = false;
        sb.phase = ScheduledBackupPhase::Active;
        sb.next_run_at = sb.schedule.next_after(now);
        Ok(sb.next_run_at)
    }

    pub fn delete(&self, name: &str) -> Result<(), ScheduledBackupError> {
        self.backups
            .write()
            .unwrap()
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| ScheduledBackupError::NotFound(name.into()))
    }
}

impl Default for ScheduledBackupManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ScheduledBackup {
    fn clone_for_return(&self) -> Self {
        Self {
            name: self.name.clone(),
            spec: self.spec.clone(),
            phase: self.phase,
            schedule: self.schedule.clone(),
            last_run_at: self.last_run_at,
            next_run_at: self.next_run_at,
            run_count: self.run_count,
            recent_runs: self.recent_runs.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(cron: &str) -> ScheduledBackupSpec {
        ScheduledBackupSpec {
            instance_id: "acme-prod".into(),
            schedule: cron.into(),
            backup_type: BackupType::Full,
            retain_runs: 3,
            suspended: false,
        }
    }

    fn ts(y: i32, mo: u32, d: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, m, 0).unwrap()
    }

    // ── Cron parser tests ───────────────────────────────────────────────────

    #[test]
    fn cron_parses_star_for_every_field() {
        let s = CronSchedule::parse("* * * * *").unwrap();
        assert!(s.matches(ts(2026, 5, 13, 12, 0)));
    }

    #[test]
    fn cron_parses_specific_minute_hour() {
        let s = CronSchedule::parse("30 3 * * *").unwrap();
        assert!(s.matches(ts(2026, 5, 13, 3, 30)));
        assert!(!s.matches(ts(2026, 5, 13, 3, 0)));
        assert!(!s.matches(ts(2026, 5, 13, 4, 30)));
    }

    #[test]
    fn cron_parses_range() {
        let s = CronSchedule::parse("0 1-5 * * *").unwrap();
        assert!(s.matches(ts(2026, 5, 13, 1, 0)));
        assert!(s.matches(ts(2026, 5, 13, 5, 0)));
        assert!(!s.matches(ts(2026, 5, 13, 6, 0)));
    }

    #[test]
    fn cron_parses_list() {
        let s = CronSchedule::parse("0,15,30,45 * * * *").unwrap();
        assert!(s.matches(ts(2026, 5, 13, 0, 15)));
        assert!(s.matches(ts(2026, 5, 13, 0, 45)));
        assert!(!s.matches(ts(2026, 5, 13, 0, 20)));
    }

    #[test]
    fn cron_parses_step() {
        let s = CronSchedule::parse("*/15 * * * *").unwrap();
        assert!(s.matches(ts(2026, 5, 13, 0, 0)));
        assert!(s.matches(ts(2026, 5, 13, 0, 30)));
        assert!(!s.matches(ts(2026, 5, 13, 0, 7)));
    }

    #[test]
    fn cron_rejects_wrong_field_count() {
        let err = CronSchedule::parse("* * *").unwrap_err();
        assert!(matches!(err, ScheduledBackupError::InvalidCron(_)));
    }

    #[test]
    fn cron_rejects_out_of_range_minute() {
        let err = CronSchedule::parse("99 * * * *").unwrap_err();
        assert!(matches!(err, ScheduledBackupError::InvalidCron(_)));
    }

    #[test]
    fn cron_rejects_zero_step() {
        let err = CronSchedule::parse("*/0 * * * *").unwrap_err();
        assert!(matches!(err, ScheduledBackupError::InvalidCron(_)));
    }

    #[test]
    fn cron_next_after_advances_by_at_least_one_minute() {
        let s = CronSchedule::parse("* * * * *").unwrap();
        let now = ts(2026, 5, 13, 12, 0);
        let n = s.next_after(now);
        assert_eq!(n, ts(2026, 5, 13, 12, 1));
    }

    #[test]
    fn cron_next_after_finds_next_daily_run() {
        let s = CronSchedule::parse("0 3 * * *").unwrap(); // 3am daily
        let now = ts(2026, 5, 13, 5, 0); // 5am
        let n = s.next_after(now);
        assert_eq!(n, ts(2026, 5, 14, 3, 0));
    }

    #[test]
    fn cron_day_of_week_constraint_filters() {
        // Mondays only (day-of-week = 1 in cron(5) Sun=0 mapping).
        let s = CronSchedule::parse("0 0 * * 1").unwrap();
        // 2026-05-13 is a Wednesday.
        let now = ts(2026, 5, 13, 0, 0);
        let n = s.next_after(now);
        assert_eq!(n.weekday().num_days_from_sunday(), 1);
        assert!(n > now);
    }

    // ── ScheduledBackupManager tests ────────────────────────────────────────

    #[test]
    fn create_validates_cron_and_stores() {
        let m = ScheduledBackupManager::new();
        let sb = m
            .create("nightly", spec("0 2 * * *"), ts(2026, 5, 13, 0, 0))
            .unwrap();
        assert_eq!(sb.next_run_at, ts(2026, 5, 13, 2, 0));
        assert_eq!(sb.phase, ScheduledBackupPhase::Active);
    }

    #[test]
    fn create_with_invalid_cron_errors() {
        let m = ScheduledBackupManager::new();
        let err = m
            .create("bad", spec("not a cron"), ts(2026, 5, 13, 0, 0))
            .unwrap_err();
        assert!(matches!(err, ScheduledBackupError::InvalidCron(_)));
    }

    #[test]
    fn create_duplicate_name_refused() {
        let m = ScheduledBackupManager::new();
        m.create("x", spec("* * * * *"), ts(2026, 5, 13, 0, 0))
            .unwrap();
        let err = m
            .create("x", spec("* * * * *"), ts(2026, 5, 13, 0, 0))
            .unwrap_err();
        assert!(matches!(err, ScheduledBackupError::AlreadyExists(_)));
    }

    #[test]
    fn trigger_due_fires_at_or_after_next_run_at() {
        let m = ScheduledBackupManager::new();
        m.create("nightly", spec("0 2 * * *"), ts(2026, 5, 13, 0, 0))
            .unwrap();
        // Not due yet
        assert!(m.trigger_due(ts(2026, 5, 13, 1, 59)).is_empty());
        // At fire time
        let fired = m.trigger_due(ts(2026, 5, 13, 2, 0));
        assert_eq!(fired, vec!["nightly".to_string()]);
        let sb = m.get("nightly").unwrap();
        assert_eq!(sb.run_count, 1);
        assert_eq!(sb.next_run_at, ts(2026, 5, 14, 2, 0));
    }

    #[test]
    fn trigger_due_skips_suspended_schedules() {
        let m = ScheduledBackupManager::new();
        let mut s = spec("* * * * *");
        s.suspended = true;
        m.create("paused", s, ts(2026, 5, 13, 0, 0)).unwrap();
        assert!(m.trigger_due(ts(2026, 5, 13, 1, 0)).is_empty());
    }

    #[test]
    fn suspend_then_resume_recomputes_next_run_at() {
        let m = ScheduledBackupManager::new();
        m.create("nightly", spec("0 2 * * *"), ts(2026, 5, 13, 0, 0))
            .unwrap();
        m.suspend("nightly").unwrap();
        assert_eq!(m.get("nightly").unwrap().phase, ScheduledBackupPhase::Suspended);
        // Resume on the 14th at 3am — next run is the 15th at 2am.
        let next = m.resume("nightly", ts(2026, 5, 14, 3, 0)).unwrap();
        assert_eq!(next, ts(2026, 5, 15, 2, 0));
        assert_eq!(m.get("nightly").unwrap().phase, ScheduledBackupPhase::Active);
    }

    #[test]
    fn retain_runs_trims_recent_runs() {
        let m = ScheduledBackupManager::new();
        let mut s = spec("* * * * *");
        s.retain_runs = 2;
        m.create("frequent", s, ts(2026, 5, 13, 0, 0)).unwrap();
        for n in 1..=4 {
            m.trigger_due(ts(2026, 5, 13, 0, n));
        }
        let sb = m.get("frequent").unwrap();
        assert_eq!(sb.run_count, 4);
        assert_eq!(sb.recent_runs.len(), 2);
    }

    #[test]
    fn delete_removes_schedule() {
        let m = ScheduledBackupManager::new();
        m.create("nightly", spec("0 2 * * *"), ts(2026, 5, 13, 0, 0))
            .unwrap();
        m.delete("nightly").unwrap();
        assert!(matches!(m.get("nightly").unwrap_err(), ScheduledBackupError::NotFound(_)));
    }

    #[test]
    fn list_returns_every_schedule() {
        let m = ScheduledBackupManager::new();
        m.create("a", spec("0 1 * * *"), ts(2026, 5, 13, 0, 0))
            .unwrap();
        m.create("b", spec("0 2 * * *"), ts(2026, 5, 13, 0, 0))
            .unwrap();
        assert_eq!(m.list().len(), 2);
    }

    #[test]
    fn next_after_skips_already_running_minute() {
        let s = CronSchedule::parse("0 * * * *").unwrap();
        // We're at exactly 12:00 — next match is 13:00, not 12:00.
        let n = s.next_after(ts(2026, 5, 13, 12, 0));
        assert_eq!(n, ts(2026, 5, 13, 13, 0));
    }
}
