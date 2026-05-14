// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Deduplication and SLA tracking — DefectDojo finding lifecycle parity.
//!
//! Cite: `dojo/finding/helper.py::do_dedupe_finding` for the dedup-key
//! contract; `dojo/sla_config/views.py::SLA_Configuration` for severity
//! → fix-deadline mapping (ADR-035 SLA: 7d/30d/90d/180d).

use crate::models::{Severity, Vulnerability};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// Deterministic deduplication key.
///
/// Cite: `do_dedupe_finding` — DefectDojo dedupes on (cve, component,
/// version-or-path) by default. Two findings from different scanners
/// for the same CVE on the same affected version collapse to one.
pub fn dedup_key(v: &Vulnerability) -> String {
    let mut key = String::new();
    key.push_str(&v.cve_id);
    key.push('|');
    key.push_str(&v.affected_component);
    key.push('|');
    let mut versions = v.affected_versions.clone();
    versions.sort();
    key.push_str(&versions.join(","));
    key
}

/// Collapse a list of findings to one per dedup-key, keeping the
/// highest-severity occurrence (CVSS-driven). Stable: input order is
/// otherwise preserved within each surviving key. Cite:
/// `dojo/finding/helper.py::deduplicate_findings`.
pub fn deduplicate(findings: Vec<Vulnerability>) -> Vec<Vulnerability> {
    let mut by_key: HashMap<String, Vulnerability> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for v in findings {
        let k = dedup_key(&v);
        match by_key.get_mut(&k) {
            Some(existing) => {
                if severity_rank(&v.severity) > severity_rank(&existing.severity)
                    || (severity_rank(&v.severity) == severity_rank(&existing.severity)
                        && v.cvss_score > existing.cvss_score)
                {
                    *existing = v;
                }
            }
            None => {
                order.push(k.clone());
                by_key.insert(k, v);
            }
        }
    }

    order.into_iter().filter_map(|k| by_key.remove(&k)).collect()
}

fn severity_rank(s: &Severity) -> u8 {
    match s {
        Severity::Critical => 4,
        Severity::High => 3,
        Severity::Medium => 2,
        Severity::Low => 1,
        Severity::Info => 0,
    }
}

/// SLA fix-deadline in days, per ADR-035: critical 7, high 30,
/// medium 90, low 180, info untracked (returns None).
pub fn sla_days(severity: &Severity) -> Option<i64> {
    match severity {
        Severity::Critical => Some(7),
        Severity::High => Some(30),
        Severity::Medium => Some(90),
        Severity::Low => Some(180),
        Severity::Info => None,
    }
}

/// Absolute SLA deadline for a finding, anchored to its `published_at`
/// timestamp. Returns `None` for severities without an SLA (Info).
/// Cite: `Finding.sla_deadline`.
pub fn sla_deadline(v: &Vulnerability) -> Option<DateTime<Utc>> {
    sla_days(&v.severity).map(|d| v.published_at + Duration::days(d))
}

/// Whether the finding has breached its SLA at `now`.
pub fn is_sla_breached(v: &Vulnerability, now: DateTime<Utc>) -> bool {
    sla_deadline(v).map_or(false, |d| now > d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VulnState;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn vuln(cve: &str, comp: &str, ver: &str, severity: Severity, cvss: f32) -> Vulnerability {
        Vulnerability {
            id: Uuid::new_v4(),
            cve_id: cve.to_string(),
            title: cve.to_string(),
            description: String::new(),
            severity,
            cvss_score: cvss,
            affected_component: comp.to_string(),
            affected_versions: vec![ver.to_string()],
            fixed_in: None,
            published_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            state: VulnState::Open,
        }
    }

    #[test]
    fn test_dedup_key_stable() {
        let a = vuln("CVE-2024-0001", "openssl", "1.0.1", Severity::High, 7.5);
        let b = vuln("CVE-2024-0001", "openssl", "1.0.1", Severity::Critical, 9.5);
        assert_eq!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn test_dedup_key_distinct_on_cve() {
        let a = vuln("CVE-2024-0001", "openssl", "1.0.1", Severity::High, 7.5);
        let b = vuln("CVE-2024-9999", "openssl", "1.0.1", Severity::High, 7.5);
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn test_dedup_key_distinct_on_component() {
        let a = vuln("CVE-2024-0001", "openssl", "1.0.1", Severity::High, 7.5);
        let b = vuln("CVE-2024-0001", "libcurl", "1.0.1", Severity::High, 7.5);
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn test_dedup_key_version_order_invariant() {
        let mut a = vuln("CVE-2024-0001", "openssl", "1.0.1", Severity::High, 7.5);
        let mut b = vuln("CVE-2024-0001", "openssl", "1.0.1", Severity::High, 7.5);
        a.affected_versions = vec!["1.0.1".into(), "1.0.2".into()];
        b.affected_versions = vec!["1.0.2".into(), "1.0.1".into()];
        assert_eq!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn test_deduplicate_collapses() {
        let findings = vec![
            vuln("CVE-2024-1", "openssl", "1.0.1", Severity::High, 7.5),
            vuln("CVE-2024-1", "openssl", "1.0.1", Severity::Critical, 9.5),
            vuln("CVE-2024-1", "openssl", "1.0.1", Severity::Medium, 5.0),
        ];
        let deduped = deduplicate(findings);
        assert_eq!(deduped.len(), 1);
        // Highest-severity wins.
        assert_eq!(deduped[0].severity, Severity::Critical);
        assert!((deduped[0].cvss_score - 9.5).abs() < 0.001);
    }

    #[test]
    fn test_deduplicate_preserves_unique() {
        let findings = vec![
            vuln("CVE-A", "openssl", "1", Severity::High, 7.5),
            vuln("CVE-B", "openssl", "1", Severity::High, 7.5),
            vuln("CVE-C", "libcurl", "2", Severity::High, 7.5),
        ];
        let deduped = deduplicate(findings);
        assert_eq!(deduped.len(), 3);
    }

    #[test]
    fn test_deduplicate_keeps_higher_cvss_when_severity_ties() {
        let findings = vec![
            vuln("CVE-X", "c", "1", Severity::High, 7.0),
            vuln("CVE-X", "c", "1", Severity::High, 8.5),
        ];
        let deduped = deduplicate(findings);
        assert_eq!(deduped.len(), 1);
        assert!((deduped[0].cvss_score - 8.5).abs() < 0.001);
    }

    #[test]
    fn test_deduplicate_preserves_first_seen_order() {
        let findings = vec![
            vuln("CVE-Z", "z", "1", Severity::High, 7.0),
            vuln("CVE-A", "a", "1", Severity::High, 7.0),
            vuln("CVE-M", "m", "1", Severity::High, 7.0),
        ];
        let deduped = deduplicate(findings);
        assert_eq!(deduped[0].cve_id, "CVE-Z");
        assert_eq!(deduped[1].cve_id, "CVE-A");
        assert_eq!(deduped[2].cve_id, "CVE-M");
    }

    #[test]
    fn test_sla_critical_7_days() {
        assert_eq!(sla_days(&Severity::Critical), Some(7));
    }

    #[test]
    fn test_sla_high_30_days() {
        assert_eq!(sla_days(&Severity::High), Some(30));
    }

    #[test]
    fn test_sla_medium_90_days() {
        assert_eq!(sla_days(&Severity::Medium), Some(90));
    }

    #[test]
    fn test_sla_low_180_days() {
        assert_eq!(sla_days(&Severity::Low), Some(180));
    }

    #[test]
    fn test_sla_info_untracked() {
        assert_eq!(sla_days(&Severity::Info), None);
    }

    #[test]
    fn test_sla_deadline_for_published() {
        let v = vuln("CVE-1", "c", "1", Severity::Critical, 9.0);
        let deadline = sla_deadline(&v).unwrap();
        let expected = v.published_at + Duration::days(7);
        assert_eq!(deadline, expected);
    }

    #[test]
    fn test_sla_deadline_none_for_info() {
        let v = vuln("CVE-1", "c", "1", Severity::Info, 0.0);
        assert!(sla_deadline(&v).is_none());
    }

    #[test]
    fn test_is_sla_breached_after_deadline() {
        let v = vuln("CVE-1", "c", "1", Severity::Critical, 9.0);
        let after = v.published_at + Duration::days(8);
        assert!(is_sla_breached(&v, after));
    }

    #[test]
    fn test_is_sla_breached_before_deadline() {
        let v = vuln("CVE-1", "c", "1", Severity::Critical, 9.0);
        let before = v.published_at + Duration::days(3);
        assert!(!is_sla_breached(&v, before));
    }

    #[test]
    fn test_is_sla_breached_info_never_breaches() {
        let v = vuln("CVE-1", "c", "1", Severity::Info, 0.0);
        let far_future = v.published_at + Duration::days(10_000);
        assert!(!is_sla_breached(&v, far_future));
    }
}
