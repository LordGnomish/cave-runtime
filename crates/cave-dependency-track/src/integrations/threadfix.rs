// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ThreadFix CSV uploader.
//!
//! ThreadFix has no first-class Dependency-Track integration; the
//! recommended bridge in the upstream community is CSV-formatted findings
//! through its `/applications/{id}/upload` endpoint.

use crate::models::{Severity, Vulnerability};

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadFixConfig {
    pub api_url: String,
    pub api_key: String,
    pub application_id: u64,
}

impl ThreadFixConfig {
    pub fn new(api_url: impl Into<String>, api_key: impl Into<String>, application_id: u64) -> Self {
        Self {
            api_url: api_url.into(),
            api_key: api_key.into(),
            application_id,
        }
    }
}

pub fn build_threadfix_csv(vulns: &[Vulnerability]) -> String {
    let mut out = String::from("vulnId,severity,cvssScore,description\n");
    for v in vulns {
        out.push_str(&format!(
            "{},{},{},{}\n",
            escape(&v.vuln_id),
            severity_str(v.severity),
            v.cvss_v3_base_score
                .map(|s| format!("{:.1}", s))
                .unwrap_or_default(),
            escape(v.description.as_deref().unwrap_or("")),
        ));
    }
    out
}

fn escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "Critical",
        Severity::High => "High",
        Severity::Medium => "Medium",
        Severity::Low => "Low",
        Severity::Info => "Info",
        Severity::Unassigned => "Info",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VulnSource;

    #[test]
    fn header_line_present() {
        let csv = build_threadfix_csv(&[]);
        assert!(csv.starts_with("vulnId,severity,cvssScore,description"));
    }

    #[test]
    fn single_vuln_rendered() {
        let mut v = Vulnerability::new("CVE-1", VulnSource::Nvd);
        v.severity = Severity::High;
        v.cvss_v3_base_score = Some(7.5);
        v.description = Some("XSS".into());
        let csv = build_threadfix_csv(&[v]);
        assert!(csv.contains("CVE-1,High,7.5,XSS"));
    }

    #[test]
    fn description_with_comma_quoted() {
        let mut v = Vulnerability::new("CVE-X", VulnSource::Nvd);
        v.description = Some("Bug, in module".into());
        let csv = build_threadfix_csv(&[v]);
        assert!(csv.contains("\"Bug, in module\""));
    }

    #[test]
    fn description_with_quote_escaped() {
        let mut v = Vulnerability::new("CVE-X", VulnSource::Nvd);
        v.description = Some("a \"b\" c".into());
        let csv = build_threadfix_csv(&[v]);
        assert!(csv.contains("\"a \"\"b\"\" c\""));
    }

    #[test]
    fn config_carries_application_id() {
        let cfg = ThreadFixConfig::new("u", "k", 7);
        assert_eq!(cfg.application_id, 7);
    }
}
