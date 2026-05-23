// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DefectDojo finding-upload bridge.
//! Mirrors `integrations.defectdojo.DefectDojoUploader`.

use crate::models::{Severity, Vulnerability};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct DefectDojoConfig {
    pub api_base: String,
    pub api_token: String,
    pub engagement_id: u64,
    pub product_name: String,
    pub scan_type: String,
}

impl DefectDojoConfig {
    pub fn new(base: impl Into<String>, token: impl Into<String>, engagement: u64) -> Self {
        Self {
            api_base: base.into(),
            api_token: token.into(),
            engagement_id: engagement,
            product_name: "cave-runtime".into(),
            scan_type: "Dependency Track Finding Packaging Format (FPF) Export".into(),
        }
    }
}

pub fn build_defectdojo_payload(cfg: &DefectDojoConfig, vulns: &[Vulnerability]) -> Value {
    let findings: Vec<Value> = vulns
        .iter()
        .map(|v| {
            json!({
                "title": v.title.clone().unwrap_or_else(|| v.vuln_id.clone()),
                "severity": severity_str(v.severity),
                "vuln_id_from_tool": v.vuln_id,
                "description": v.description.clone().unwrap_or_default(),
                "cvssv3": v.cvss_v3_vector.clone().unwrap_or_default(),
                "cvssv3_score": v.cvss_v3_base_score,
                "cwe": v.cwes.first().copied().unwrap_or(0),
                "active": true,
                "verified": false,
            })
        })
        .collect();
    json!({
        "engagement": cfg.engagement_id,
        "product_name": cfg.product_name,
        "scan_type": cfg.scan_type,
        "findings": findings,
    })
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
    fn payload_carries_engagement_and_findings() {
        let cfg = DefectDojoConfig::new("https://dd", "token", 42);
        let mut v = Vulnerability::new("CVE-1", VulnSource::Nvd);
        v.severity = Severity::High;
        v.title = Some("XSS".into());
        let p = build_defectdojo_payload(&cfg, &[v]);
        assert_eq!(p["engagement"], 42);
        assert_eq!(p["findings"][0]["severity"], "High");
        assert_eq!(p["findings"][0]["title"], "XSS");
    }

    #[test]
    fn severity_map_full() {
        assert_eq!(severity_str(Severity::Critical), "Critical");
        assert_eq!(severity_str(Severity::Unassigned), "Info");
    }

    #[test]
    fn empty_vulns_yields_empty_findings() {
        let cfg = DefectDojoConfig::new("u", "t", 1);
        let p = build_defectdojo_payload(&cfg, &[]);
        assert_eq!(p["findings"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn scan_type_matches_upstream_default() {
        let cfg = DefectDojoConfig::new("u", "t", 1);
        assert!(cfg.scan_type.starts_with("Dependency Track"));
    }
}
