// SPDX-License-Identifier: AGPL-3.0-or-later
//! Legacy dedup + SLA functions kept for backwards compatibility
//! with the original `models::Vulnerability` shape.
//!
//! These predate the DefectDojo-parity port (see sibling `mod.rs`).
//! Source: ADR-035 internal SLA windows, the original cave-vulns scaffold
//! committed before the deep port.

use crate::models::{Severity, Vulnerability};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// Deterministic deduplication key (legacy `Vulnerability`).
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

/// Collapse a list of findings to one per dedup-key, highest-severity wins.
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

/// SLA fix-deadline in days, per ADR-035.
pub fn sla_days(severity: &Severity) -> Option<i64> {
    match severity {
        Severity::Critical => Some(7),
        Severity::High => Some(30),
        Severity::Medium => Some(90),
        Severity::Low => Some(180),
        Severity::Info => None,
    }
}

pub fn sla_deadline(v: &Vulnerability) -> Option<DateTime<Utc>> {
    sla_days(&v.severity).map(|d| v.published_at + Duration::days(d))
}

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
    fn dedup_key_stable() {
        let a = vuln("CVE-2024-1", "openssl", "1.0.1", Severity::High, 7.5);
        let b = vuln("CVE-2024-1", "openssl", "1.0.1", Severity::Critical, 9.5);
        assert_eq!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn deduplicate_collapses_to_highest_severity() {
        let findings = vec![
            vuln("CVE-1", "openssl", "1.0.1", Severity::High, 7.5),
            vuln("CVE-1", "openssl", "1.0.1", Severity::Critical, 9.5),
        ];
        let out = deduplicate(findings);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Critical);
    }

    #[test]
    fn sla_critical_seven_days() { assert_eq!(sla_days(&Severity::Critical), Some(7)); }
    #[test]
    fn sla_high_thirty_days() { assert_eq!(sla_days(&Severity::High), Some(30)); }
    #[test]
    fn sla_medium_ninety_days() { assert_eq!(sla_days(&Severity::Medium), Some(90)); }
    #[test]
    fn sla_low_one_eighty_days() { assert_eq!(sla_days(&Severity::Low), Some(180)); }
    #[test]
    fn sla_info_untracked() { assert_eq!(sla_days(&Severity::Info), None); }

    #[test]
    fn sla_breach_detection() {
        let v = vuln("CVE-1", "c", "1", Severity::Critical, 9.0);
        let after = v.published_at + Duration::days(8);
        assert!(is_sla_breached(&v, after));
        let before = v.published_at + Duration::days(3);
        assert!(!is_sla_breached(&v, before));
    }
}
