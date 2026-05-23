// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Filter pipeline — severity gate + fixed-status filter + ignore policy.
//!
//! Mirrors trivy's `pkg/result/filter`. Filters mutate a `Report` in place
//! and return the suppressed count, so policy decisions can be audited.

use crate::ignore::IgnorePolicy;
use crate::models::Report;
use crate::severity::Severity;

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub min_severity: Option<Severity>,
    pub only_fixed: bool,
    pub ignore_unfixed: bool,
    pub ignore: Option<IgnorePolicy>,
}

impl Filter {
    pub fn min_severity(mut self, s: Severity) -> Self {
        self.min_severity = Some(s);
        self
    }
    pub fn only_fixed(mut self) -> Self {
        self.only_fixed = true;
        self
    }
    pub fn ignore_unfixed(mut self) -> Self {
        self.ignore_unfixed = true;
        self
    }
    pub fn with_ignore(mut self, p: IgnorePolicy) -> Self {
        self.ignore = Some(p);
        self
    }

    /// Apply the filter to a report, returning total suppressed vulns.
    pub fn apply(&self, report: &mut Report) -> usize {
        let mut suppressed = 0usize;
        for r in &mut report.results {
            let before = r.vulnerabilities.len();
            r.vulnerabilities.retain(|v| {
                if let Some(min) = self.min_severity {
                    if !v.severity.at_least(min) {
                        return false;
                    }
                }
                if self.only_fixed && v.fixed_version.is_none() {
                    return false;
                }
                if self.ignore_unfixed && v.fixed_version.is_none() {
                    return false;
                }
                if let Some(ig) = &self.ignore {
                    if ig.matches_id(&v.id) {
                        return false;
                    }
                }
                true
            });
            // Also filter misconfigs by severity and ignore policy.
            r.misconfigurations.retain(|m| {
                if let Some(min) = self.min_severity {
                    if !m.severity.at_least(min) {
                        return false;
                    }
                }
                if let Some(ig) = &self.ignore {
                    if ig.matches_id(&m.id) {
                        return false;
                    }
                }
                true
            });
            // And secrets by severity.
            r.secrets.retain(|s| {
                if let Some(min) = self.min_severity {
                    if !s.severity.at_least(min) {
                        return false;
                    }
                }
                true
            });
            suppressed += before - r.vulnerabilities.len();
        }
        suppressed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Report, ScanResult, Vulnerability};

    fn rep() -> Report {
        let mut r = Report::new("x", "container_image");
        let mut sr = ScanResult {
            target: "x".into(),
            class: "os-pkgs".into(),
            ..Default::default()
        };
        sr.vulnerabilities.push(Vulnerability {
            id: "CVE-1".into(),
            pkg_name: "p".into(),
            installed_version: "1".into(),
            fixed_version: Some("2".into()),
            severity: Severity::Critical,
            references: vec![],
            title: None,
        });
        sr.vulnerabilities.push(Vulnerability {
            id: "CVE-2".into(),
            pkg_name: "q".into(),
            installed_version: "1".into(),
            fixed_version: None,
            severity: Severity::Low,
            references: vec![],
            title: None,
        });
        r.results.push(sr);
        r
    }

    #[test]
    fn min_severity_filter() {
        let mut r = rep();
        let f = Filter::default().min_severity(Severity::High);
        f.apply(&mut r);
        assert_eq!(r.total_vulns(), 1);
    }

    #[test]
    fn only_fixed() {
        let mut r = rep();
        let f = Filter::default().only_fixed();
        f.apply(&mut r);
        assert_eq!(r.total_vulns(), 1);
    }

    #[test]
    fn ignore_id() {
        let mut r = rep();
        let mut policy = IgnorePolicy::default();
        policy.add("CVE-1");
        let f = Filter::default().with_ignore(policy);
        f.apply(&mut r);
        assert_eq!(r.total_vulns(), 1);
        assert_eq!(r.results[0].vulnerabilities[0].id, "CVE-2");
    }

    #[test]
    fn ignore_unfixed() {
        let mut r = rep();
        let f = Filter::default().ignore_unfixed();
        let suppressed = f.apply(&mut r);
        assert_eq!(suppressed, 1);
    }

    #[test]
    fn returns_suppression_count() {
        let mut r = rep();
        let f = Filter::default().min_severity(Severity::Critical);
        let n = f.apply(&mut r);
        assert_eq!(n, 1);
    }
}
