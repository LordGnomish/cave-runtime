// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Offline vulnerability database.
//!
//! Mirrors trivy's `pkg/db` + `pkg/vulnerability` ingest paths for CVE, GHSA
//! and OSV advisories. The in-memory representation indexes by ecosystem +
//! package name and supports range matching ("introduced..fixed") via
//! a lexicographic version comparator that matches Go's
//! `pkg/version/version.go` for the formats cave-trivy MVP needs (semver,
//! debian, rpm-ish). Online refresh + sled persistence are scope cuts —
//! see parity.manifest.toml.

use crate::error::{TrivyError, TrivyResult};
use crate::osv::OsvAdvisory;
use crate::severity::Severity;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryEntry {
    pub id: String,
    pub aliases: Vec<String>,
    pub ecosystem: String,
    pub pkg: String,
    pub introduced: Option<String>,
    pub fixed: Option<String>,
    pub severity: Severity,
    pub references: Vec<String>,
    pub title: String,
}

#[derive(Debug, Clone, Default)]
pub struct VulnDb {
    by_eco_pkg: HashMap<(String, String), Vec<AdvisoryEntry>>,
    by_id: HashMap<String, AdvisoryEntry>,
    pub source: String,
    pub built_at: String,
}

impl VulnDb {
    pub fn new() -> Self {
        Self::default()
    }

    /// In-memory fixture used by smoke + self-audit. Contains a few known
    /// CVEs across alpine, debian, npm, pypi, cargo so tests don't require
    /// any network or local file fixture.
    pub fn cave_default() -> Self {
        let mut db = Self::new();
        db.source = "cave-default".into();
        db.built_at = "2026-05-23T00:00:00Z".into();
        for e in cave_default_fixture() {
            db.insert(e);
        }
        db
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn insert(&mut self, e: AdvisoryEntry) {
        let key = (e.ecosystem.clone(), e.pkg.clone());
        self.by_id.insert(e.id.clone(), e.clone());
        self.by_eco_pkg.entry(key).or_default().push(e);
    }

    pub fn lookup_id(&self, id: &str) -> Option<&AdvisoryEntry> {
        self.by_id.get(id)
    }

    pub fn lookup(&self, ecosystem: &str, pkg: &str) -> &[AdvisoryEntry] {
        self.by_eco_pkg
            .get(&(ecosystem.to_string(), pkg.to_string()))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn ingest_osv(&mut self, advisories: &[OsvAdvisory]) -> TrivyResult<usize> {
        let mut n = 0;
        for adv in advisories {
            for (eco, name, intro, fixed) in adv.affected_tuples() {
                let entry = AdvisoryEntry {
                    id: adv.id.clone(),
                    aliases: adv.aliases.clone(),
                    ecosystem: eco,
                    pkg: name,
                    introduced: intro,
                    fixed,
                    severity: cvss_to_severity(adv.primary_score_kind(), &adv.severity),
                    references: adv.references.iter().map(|r| r.url.clone()).collect(),
                    title: adv.summary.clone(),
                };
                self.insert(entry);
                n += 1;
            }
        }
        if n == 0 {
            return Err(TrivyError::VulnDb("no advisories ingested".into()));
        }
        Ok(n)
    }

    /// Find all advisories that affect `installed_version` for (ecosystem, pkg).
    pub fn match_pkg(&self, ecosystem: &str, pkg: &str, installed_version: &str) -> Vec<&AdvisoryEntry> {
        let mut out = Vec::new();
        for entry in self.lookup(ecosystem, pkg) {
            if version_in_range(installed_version, entry.introduced.as_deref(), entry.fixed.as_deref()) {
                out.push(entry);
            }
        }
        out
    }
}

/// Trivy's range model: introduced <= v < fixed. Either bound may be `None`.
pub fn version_in_range(v: &str, introduced: Option<&str>, fixed: Option<&str>) -> bool {
    if let Some(i) = introduced {
        if compare_versions(v, i) < 0 {
            return false;
        }
    }
    if let Some(f) = fixed {
        if compare_versions(v, f) >= 0 {
            return false;
        }
    }
    true
}

/// Lexicographic-by-component version compare. Numeric components compared
/// numerically; non-numeric fall back to lexicographic. Mirrors what trivy's
/// `pkg/version/version.go` does for the SEMVER / DEB / RPM intersection
/// that cave-trivy MVP supports.
pub fn compare_versions(a: &str, b: &str) -> i32 {
    let split = |s: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut cur_is_num = false;
        for c in s.chars() {
            let is_num = c.is_ascii_digit();
            let is_sep = matches!(c, '.' | '-' | '+' | '~' | '_');
            if is_sep {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur_is_num = false;
                continue;
            }
            if !cur.is_empty() && is_num != cur_is_num {
                out.push(std::mem::take(&mut cur));
            }
            cur_is_num = is_num;
            cur.push(c);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    };
    let ap = split(a);
    let bp = split(b);
    let n = ap.len().max(bp.len());
    for i in 0..n {
        let ai = ap.get(i).map(String::as_str).unwrap_or("0");
        let bi = bp.get(i).map(String::as_str).unwrap_or("0");
        let a_num: Option<u64> = ai.parse().ok();
        let b_num: Option<u64> = bi.parse().ok();
        let ord = match (a_num, b_num) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => ai.cmp(bi),
        };
        match ord {
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Equal => {}
        }
    }
    0
}

fn cvss_to_severity(_kind: Option<&str>, severities: &[crate::osv::OsvSeverity]) -> Severity {
    let mut max = 0.0f64;
    for s in severities {
        if let Ok(v) = s.score.parse::<f64>() {
            if v > max {
                max = v;
            }
        }
    }
    match max {
        x if x >= 9.0 => Severity::Critical,
        x if x >= 7.0 => Severity::High,
        x if x >= 4.0 => Severity::Medium,
        x if x > 0.0 => Severity::Low,
        _ => Severity::Unknown,
    }
}

fn cave_default_fixture() -> Vec<AdvisoryEntry> {
    let mk = |id, eco: &str, pkg: &str, intro: Option<&str>, fixed: Option<&str>, sev| AdvisoryEntry {
        id: String::from(id),
        aliases: vec![],
        ecosystem: eco.into(),
        pkg: pkg.into(),
        introduced: intro.map(String::from),
        fixed: fixed.map(String::from),
        severity: sev,
        references: vec![format!("https://nvd.nist.gov/vuln/detail/{}", id)],
        title: format!("{} affects {}/{}", id, eco, pkg),
    };
    vec![
        mk("CVE-2026-0001", "alpine", "openssl", Some("0"), Some("3.0.13"), Severity::Critical),
        mk("CVE-2026-0002", "alpine", "musl",    Some("0"), Some("1.2.5"),  Severity::Medium),
        mk("CVE-2026-0003", "debian", "openssl", Some("0"), Some("3.0.13-1"), Severity::High),
        mk("CVE-2026-0004", "ubuntu", "curl",    Some("0"), Some("8.5.0-1"),  Severity::High),
        mk("CVE-2026-0005", "rhel",   "kernel",  Some("0"), Some("5.14.0-500"), Severity::Critical),
        mk("CVE-2026-0010", "npm",    "lodash",  Some("0"), Some("4.17.21"), Severity::High),
        mk("CVE-2026-0011", "npm",    "express", Some("0"), Some("4.19.2"),  Severity::Medium),
        mk("CVE-2026-0020", "pypi",   "requests",Some("0"), Some("2.32.0"),  Severity::Medium),
        mk("CVE-2026-0021", "pypi",   "django",  Some("0"), Some("5.0.4"),   Severity::High),
        mk("CVE-2026-0030", "cargo",  "openssl-sys", Some("0"), Some("0.9.100"), Severity::Medium),
        mk("CVE-2026-0040", "go",     "github.com/golang-jwt/jwt", Some("0"), Some("4.5.1"), Severity::High),
        mk("CVE-2026-0050", "maven",  "org.springframework:spring-core", Some("0"), Some("6.1.6"), Severity::Critical),
        mk("CVE-2026-0060", "gem",    "actionpack", Some("0"), Some("7.1.3"), Severity::High),
        mk("CVE-2026-0070", "composer", "symfony/http-foundation", Some("0"), Some("6.4.5"), Severity::Medium),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_cmp_numeric() {
        assert_eq!(compare_versions("1.2.3", "1.2.3"), 0);
        assert_eq!(compare_versions("1.2.3", "1.2.4"), -1);
        assert_eq!(compare_versions("1.2.10", "1.2.2"), 1);
        assert_eq!(compare_versions("2.0.0", "1.99.99"), 1);
    }

    #[test]
    fn version_cmp_mixed() {
        // alpha < beta < numeric
        assert_eq!(compare_versions("1.0.0a", "1.0.0b"), -1);
        // "3.0.13-1" > "3.0.12-9"
        assert_eq!(compare_versions("3.0.13-1", "3.0.12-9"), 1);
    }

    #[test]
    fn range_inclusive_introduced_exclusive_fixed() {
        assert!(version_in_range("3.0.0", Some("0"), Some("3.0.13")));
        assert!(!version_in_range("3.0.13", Some("0"), Some("3.0.13")));
        assert!(!version_in_range("3.0.14", Some("0"), Some("3.0.13")));
        assert!(version_in_range("9.9.9", None, None));
        assert!(version_in_range("9.9.9", Some("0"), None));
        assert!(!version_in_range("0.0.0", Some("1.0.0"), None));
    }

    #[test]
    fn default_db_populates() {
        let db = VulnDb::cave_default();
        assert!(db.len() >= 14);
        assert!(db.lookup_id("CVE-2026-0001").is_some());
        assert!(db.lookup("alpine", "openssl").iter().any(|e| e.id == "CVE-2026-0001"));
    }

    #[test]
    fn match_pkg_returns_in_range() {
        let db = VulnDb::cave_default();
        let v = db.match_pkg("alpine", "openssl", "3.0.0");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "CVE-2026-0001");
        let v2 = db.match_pkg("alpine", "openssl", "9.9.9");
        assert!(v2.is_empty());
    }

    #[test]
    fn ingest_osv_populates() {
        let mut db = VulnDb::new();
        let adv = OsvAdvisory {
            id: "GHSA-xxxx".into(),
            affected: vec![crate::osv::OsvAffected {
                package: crate::osv::OsvPackage {
                    ecosystem: "npm".into(),
                    name: "leftpad".into(),
                    purl: "".into(),
                },
                ranges: vec![crate::osv::OsvRange {
                    kind: "SEMVER".into(),
                    events: vec![
                        crate::osv::OsvEvent {
                            introduced: Some("0".into()),
                            fixed: None,
                            last_affected: None,
                        },
                        crate::osv::OsvEvent {
                            introduced: None,
                            fixed: Some("1.0.0".into()),
                            last_affected: None,
                        },
                    ],
                }],
                versions: vec![],
            }],
            severity: vec![crate::osv::OsvSeverity {
                kind: "CVSS_V3".into(),
                score: "8.1".into(),
            }],
            ..Default::default()
        };
        let n = db.ingest_osv(&[adv]).unwrap();
        assert_eq!(n, 1);
        assert!(db.lookup_id("GHSA-xxxx").is_some());
        assert_eq!(db.lookup("npm", "leftpad")[0].severity, Severity::High);
    }

    #[test]
    fn ingest_empty_fails() {
        let mut db = VulnDb::new();
        assert!(db.ingest_osv(&[]).is_err());
    }

    #[test]
    fn cvss_thresholds() {
        let mk = |score: &str| vec![crate::osv::OsvSeverity { kind: "CVSS_V3".into(), score: score.into() }];
        assert_eq!(cvss_to_severity(None, &mk("9.5")), Severity::Critical);
        assert_eq!(cvss_to_severity(None, &mk("7.5")), Severity::High);
        assert_eq!(cvss_to_severity(None, &mk("5.0")), Severity::Medium);
        assert_eq!(cvss_to_severity(None, &mk("1.0")), Severity::Low);
        assert_eq!(cvss_to_severity(None, &[]), Severity::Unknown);
    }
}
