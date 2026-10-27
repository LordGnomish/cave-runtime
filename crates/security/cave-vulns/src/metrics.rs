// SPDX-License-Identifier: AGPL-3.0-or-later
//! Findings metrics aggregation — severity tallies, open/closed/accepted
//! status partition, remediation-age buckets, and time-windowed
//! opened-trend rollups.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738
//!         dojo/metrics/utils.py (`severity_count`, `findings_queryset`,
//!         age bucketing) and dojo/metrics/views.py (period rollups).
//!
//! Upstream computes these as Django ORM annotations over querysets; we
//! port the same arithmetic as pure functions over `&[Finding]` so the
//! contract is deterministic and unit-testable without a database. The
//! product/engineer breakdowns that require the Product→Engagement→Test
//! join live in `hierarchy` and are intentionally out of scope here.

use crate::finding::{Finding, FindingSeverity};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};

/// Per-severity counts plus a grand total. Mirrors `severity_count()`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeverityCounts {
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub info: u32,
    pub total: u32,
}

impl SeverityCounts {
    /// Tally a set of findings by severity.
    pub fn tally<'a, I: IntoIterator<Item = &'a Finding>>(_findings: I) -> Self {
        Self::default() // stub — GREEN commit
    }
}

/// Open / closed / accepted partition. Each finding falls in exactly one
/// bucket with precedence closed > accepted > open, matching how upstream's
/// CLOSED/ACCEPTED/OPEN querysets are applied in sequence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusCounts {
    pub open: u32,
    pub closed: u32,
    pub accepted: u32,
}

impl StatusCounts {
    pub fn tally(_findings: &[Finding]) -> Self {
        Self::default() // stub
    }
}

/// Remediation-age histogram (days since discovery, capped at mitigation
/// time or `now`). Buckets match upstream: 0-30, 31-60, 61-90, 91+.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgeBuckets {
    pub under_30: u32,
    pub d31_60: u32,
    pub d61_90: u32,
    pub d90_plus: u32,
}

/// Bucket findings by remediation age. Open findings age against `now`;
/// mitigated findings age against their `mitigated` timestamp.
pub fn age_buckets(_findings: &[Finding], _now: DateTime<Utc>) -> AgeBuckets {
    AgeBuckets::default() // stub
}

/// Count findings whose discovery `date` lands in `[start, end)`.
pub fn opened_in_period(
    _findings: &[Finding],
    _start: DateTime<Utc>,
    _end: DateTime<Utc>,
) -> u32 {
    0 // stub
}

/// One calendar-month bucket of opened findings, tallied by severity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthBucket {
    pub year: i32,
    pub month: u32,
    pub counts: SeverityCounts,
}

/// Monthly opened-trend rollup across `[start, end]` inclusive of both
/// endpoints' months, zero-filling months with no findings.
pub fn monthly_opened(
    _findings: &[Finding],
    _start: DateTime<Utc>,
    _end: DateTime<Utc>,
) -> Vec<MonthBucket> {
    Vec::new() // stub
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    fn finding(sev: FindingSeverity, date: DateTime<Utc>) -> Finding {
        let mut f = Finding::new("t", sev);
        f.date = date;
        f
    }

    #[test]
    fn severity_count_one_of_each() {
        let fs = vec![
            finding(FindingSeverity::Critical, at(2024, 1, 1)),
            finding(FindingSeverity::High, at(2024, 1, 1)),
            finding(FindingSeverity::Medium, at(2024, 1, 1)),
            finding(FindingSeverity::Low, at(2024, 1, 1)),
            finding(FindingSeverity::Info, at(2024, 1, 1)),
        ];
        let c = SeverityCounts::tally(&fs);
        assert_eq!(
            c,
            SeverityCounts {
                critical: 1,
                high: 1,
                medium: 1,
                low: 1,
                info: 1,
                total: 5
            }
        );
    }

    #[test]
    fn severity_count_groups_duplicates() {
        let fs = vec![
            finding(FindingSeverity::High, at(2024, 1, 1)),
            finding(FindingSeverity::High, at(2024, 1, 1)),
            finding(FindingSeverity::Critical, at(2024, 1, 1)),
        ];
        let c = SeverityCounts::tally(&fs);
        assert_eq!(c.high, 2);
        assert_eq!(c.critical, 1);
        assert_eq!(c.total, 3);
    }

    #[test]
    fn status_partition_open_closed_accepted() {
        let mut open = finding(FindingSeverity::High, at(2024, 1, 1));
        open.state.active = true;
        let mut closed = finding(FindingSeverity::High, at(2024, 1, 1));
        closed.state.is_mitigated = true;
        closed.state.active = false;
        let mut accepted = finding(FindingSeverity::High, at(2024, 1, 1));
        accepted.state.risk_accepted = true;
        accepted.state.active = false;
        let fs = vec![open, closed, accepted];
        let s = StatusCounts::tally(&fs);
        assert_eq!(s, StatusCounts { open: 1, closed: 1, accepted: 1 });
    }

    #[test]
    fn status_precedence_closed_wins_over_accepted() {
        // A finding both mitigated and risk-accepted counts as closed only.
        let mut f = finding(FindingSeverity::Medium, at(2024, 1, 1));
        f.state.is_mitigated = true;
        f.state.risk_accepted = true;
        let s = StatusCounts::tally(&[f]);
        assert_eq!(s, StatusCounts { open: 0, closed: 1, accepted: 0 });
    }

    #[test]
    fn age_buckets_open_findings_against_now() {
        let now = at(2024, 5, 30);
        // 135 days old → 90+, 75 days → 61-90, 45 days → 31-60, 10 days → <30
        let fs = vec![
            finding(FindingSeverity::Critical, at(2024, 1, 16)), // ~135d
            finding(FindingSeverity::High, at(2024, 3, 16)),     // ~75d
            finding(FindingSeverity::High, at(2024, 4, 15)),     // ~45d
            finding(FindingSeverity::Low, at(2024, 5, 20)),      // 10d
        ];
        let b = age_buckets(&fs, now);
        assert_eq!(b.under_30, 1);
        assert_eq!(b.d31_60, 1);
        assert_eq!(b.d61_90, 1);
        assert_eq!(b.d90_plus, 1);
    }

    #[test]
    fn age_buckets_mitigated_uses_mitigated_time() {
        let now = at(2024, 12, 31);
        let mut f = finding(FindingSeverity::High, at(2024, 1, 1));
        // Mitigated 20 days after discovery → bucket by 20d not full year.
        f.state.is_mitigated = true;
        f.mitigated = Some(at(2024, 1, 21));
        let b = age_buckets(&[f], now);
        assert_eq!(b.under_30, 1);
        assert_eq!(b.d90_plus, 0);
    }

    #[test]
    fn opened_in_period_is_half_open_interval() {
        let fs = vec![
            finding(FindingSeverity::Low, at(2024, 1, 1)),  // in
            finding(FindingSeverity::Low, at(2024, 1, 31)), // in
            finding(FindingSeverity::Low, at(2024, 2, 1)),  // out (== end)
            finding(FindingSeverity::Low, at(2023, 12, 31)), // out (< start)
        ];
        let n = opened_in_period(&fs, at(2024, 1, 1), at(2024, 2, 1));
        assert_eq!(n, 2);
    }

    #[test]
    fn monthly_opened_zero_fills_missing_months() {
        let fs = vec![
            finding(FindingSeverity::Critical, at(2024, 1, 10)),
            finding(FindingSeverity::High, at(2024, 3, 5)),
            finding(FindingSeverity::High, at(2024, 3, 25)),
        ];
        let buckets = monthly_opened(&fs, at(2024, 1, 1), at(2024, 3, 31));
        // Jan, Feb, Mar — three months, Feb zero-filled.
        assert_eq!(buckets.len(), 3);
        assert_eq!((buckets[0].year, buckets[0].month), (2024, 1));
        assert_eq!(buckets[0].counts.critical, 1);
        assert_eq!(buckets[0].counts.total, 1);
        assert_eq!((buckets[1].year, buckets[1].month), (2024, 2));
        assert_eq!(buckets[1].counts.total, 0);
        assert_eq!((buckets[2].year, buckets[2].month), (2024, 3));
        assert_eq!(buckets[2].counts.high, 2);
        assert_eq!(buckets[2].counts.total, 2);
    }

    #[test]
    fn monthly_opened_spans_year_boundary() {
        let fs = vec![finding(FindingSeverity::Low, at(2024, 12, 15))];
        let buckets = monthly_opened(&fs, at(2024, 11, 1), at(2025, 1, 31));
        assert_eq!(buckets.len(), 3); // Nov, Dec, Jan
        assert_eq!((buckets[0].year, buckets[0].month), (2024, 11));
        assert_eq!((buckets[2].year, buckets[2].month), (2025, 1));
        assert_eq!(buckets[1].counts.low, 1); // Dec
    }
}
