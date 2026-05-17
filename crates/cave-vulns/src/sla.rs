// SPDX-License-Identifier: AGPL-3.0-or-later
//! SLA tracking — per-severity remediation windows, breach detection,
//! days-until-breach metric.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py:999
//!         (`class SLA_Configuration`).  DefectDojo lets each Product
//!         override the global SLA window; this port mirrors that.

use crate::finding::{Finding, FindingSeverity};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One SLA configuration — owns the per-severity day-counts.
///
/// Source: dojo/models.py:999 (`class SLA_Configuration`).
/// Default windows match dojo/models.py defaults
/// (Critical:7 / High:30 / Medium:90 / Low:120).
/// Cave's defaults follow ADR-035 (Low:180 for parity with our policy).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlaConfiguration {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub critical: u32,
    pub enforce_critical: bool,
    pub high: u32,
    pub enforce_high: bool,
    pub medium: u32,
    pub enforce_medium: bool,
    pub low: u32,
    pub enforce_low: bool,
}

impl Default for SlaConfiguration {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Default".into(),
            description: None,
            // ADR-035 defaults; DefectDojo upstream: 7/30/90/120.
            critical: 7,
            enforce_critical: true,
            high: 30,
            enforce_high: true,
            medium: 90,
            enforce_medium: true,
            low: 180,
            enforce_low: true,
        }
    }
}

impl SlaConfiguration {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Self::default() }
    }

    /// Window in days for a severity. `None` for Info or when the
    /// severity has `enforce_*=false`.
    /// Source: dojo/models.py:999 — `SLA_Configuration.get_breach_days_by_severity`.
    pub fn days_for(&self, sev: FindingSeverity) -> Option<i64> {
        match sev {
            FindingSeverity::Critical if self.enforce_critical => Some(self.critical as i64),
            FindingSeverity::High if self.enforce_high => Some(self.high as i64),
            FindingSeverity::Medium if self.enforce_medium => Some(self.medium as i64),
            FindingSeverity::Low if self.enforce_low => Some(self.low as i64),
            _ => None,
        }
    }

    /// Absolute SLA deadline anchored to `f.date` (DefectDojo's
    /// `sla_start_date` falls back to `date`). Source: dojo/models.py:2453.
    pub fn deadline(&self, f: &Finding) -> Option<DateTime<Utc>> {
        self.days_for(f.severity).map(|d| f.date + Duration::days(d))
    }

    /// Days until SLA breach. Negative = already breached.
    /// `None` when the severity isn't tracked.
    pub fn days_until_breach(&self, f: &Finding, now: DateTime<Utc>) -> Option<i64> {
        self.deadline(f).map(|d| (d - now).num_days())
    }

    /// True iff `now` is past the deadline.
    /// Source: dojo/finding/views.py::SLA logic.
    pub fn is_breached(&self, f: &Finding, now: DateTime<Utc>) -> bool {
        self.deadline(f).map_or(false, |d| now > d)
    }
}

/// Roll up breach counts across many findings.
pub struct SlaReport {
    pub total: usize,
    pub breached: usize,
    pub breaching_soon: usize, // ≤ 7 days
    pub by_severity: Vec<(FindingSeverity, usize, usize)>, // (sev, total, breached)
}

pub fn rollup(cfg: &SlaConfiguration, findings: &[Finding], now: DateTime<Utc>) -> SlaReport {
    let mut total = 0usize;
    let mut breached = 0usize;
    let mut breaching_soon = 0usize;
    let mut buckets: Vec<(FindingSeverity, usize, usize)> = vec![
        (FindingSeverity::Critical, 0, 0),
        (FindingSeverity::High, 0, 0),
        (FindingSeverity::Medium, 0, 0),
        (FindingSeverity::Low, 0, 0),
        (FindingSeverity::Info, 0, 0),
    ];
    for f in findings {
        if !f.state.active {
            continue;
        }
        total += 1;
        let bucket = buckets.iter_mut().find(|(s, _, _)| *s == f.severity).unwrap();
        bucket.1 += 1;
        if cfg.is_breached(f, now) {
            breached += 1;
            bucket.2 += 1;
        } else if let Some(days) = cfg.days_until_breach(f, now) {
            if days <= 7 {
                breaching_soon += 1;
            }
        }
    }
    SlaReport { total, breached, breaching_soon, by_severity: buckets }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fin(sev: FindingSeverity, days_ago: i64) -> Finding {
        let mut f = Finding::new("X", sev);
        f.date = Utc::now() - Duration::days(days_ago);
        f
    }

    #[test]
    fn default_windows_match_charter() {
        let cfg = SlaConfiguration::default();
        assert_eq!(cfg.days_for(FindingSeverity::Critical), Some(7));
        assert_eq!(cfg.days_for(FindingSeverity::High), Some(30));
        assert_eq!(cfg.days_for(FindingSeverity::Medium), Some(90));
        assert_eq!(cfg.days_for(FindingSeverity::Low), Some(180));
        assert_eq!(cfg.days_for(FindingSeverity::Info), None);
    }

    #[test]
    fn disabled_enforcement_returns_none() {
        let mut cfg = SlaConfiguration::default();
        cfg.enforce_critical = false;
        assert_eq!(cfg.days_for(FindingSeverity::Critical), None);
    }

    #[test]
    fn deadline_for_finding_anchored_to_date() {
        let cfg = SlaConfiguration::default();
        let f = fin(FindingSeverity::High, 0);
        let dl = cfg.deadline(&f).unwrap();
        let expected = f.date + Duration::days(30);
        let delta = (dl - expected).num_seconds().abs();
        assert!(delta < 5);
    }

    #[test]
    fn is_breached_after_window() {
        let cfg = SlaConfiguration::default();
        let f = fin(FindingSeverity::Critical, 30);
        assert!(cfg.is_breached(&f, Utc::now()));
    }

    #[test]
    fn is_not_breached_within_window() {
        let cfg = SlaConfiguration::default();
        let f = fin(FindingSeverity::High, 5);
        assert!(!cfg.is_breached(&f, Utc::now()));
    }

    #[test]
    fn days_until_breach_negative_when_late() {
        let cfg = SlaConfiguration::default();
        let f = fin(FindingSeverity::Critical, 10);
        let d = cfg.days_until_breach(&f, Utc::now()).unwrap();
        assert!(d < 0, "got {d}");
    }

    #[test]
    fn rollup_buckets_by_severity_active_only() {
        let cfg = SlaConfiguration::default();
        let mut findings = vec![
            fin(FindingSeverity::Critical, 30),
            fin(FindingSeverity::High, 5),
            fin(FindingSeverity::Medium, 200),
            fin(FindingSeverity::Info, 1000),
        ];
        // Mark medium as mitigated → excluded.
        findings[2].state.active = false;
        let r = rollup(&cfg, &findings, Utc::now());
        assert_eq!(r.total, 3); // 4 findings, 1 inactive
        assert_eq!(r.breached, 1); // only the critical
        let crit_bucket = r.by_severity.iter().find(|b| b.0 == FindingSeverity::Critical).unwrap();
        assert_eq!(crit_bucket.1, 1);
        assert_eq!(crit_bucket.2, 1);
    }

    #[test]
    fn breaching_soon_captures_7_day_window() {
        let cfg = SlaConfiguration::default();
        let f = fin(FindingSeverity::Critical, 5); // 7-day window, 5 days in → 2 days left
        let r = rollup(&cfg, &[f], Utc::now());
        assert_eq!(r.breaching_soon, 1);
    }
}
