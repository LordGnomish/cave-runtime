// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Fortify SSC integration.  Mirrors `integrations.fortifyssc.FortifySscUploader`.

use crate::models::Vulnerability;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct FortifyConfig {
    pub ssc_url: String,
    pub citoken: String,
    pub application_version_id: u64,
}

impl FortifyConfig {
    pub fn new(ssc_url: impl Into<String>, citoken: impl Into<String>, av_id: u64) -> Self {
        Self {
            ssc_url: ssc_url.into(),
            citoken: citoken.into(),
            application_version_id: av_id,
        }
    }
}

/// Builds the Fortify "external-list" payload — one item per vulnerability,
/// keyed by `vuln_id` and tagged with the cave-runtime application version.
pub fn build_fortify_payload(cfg: &FortifyConfig, vulns: &[Vulnerability]) -> Value {
    let items: Vec<Value> = vulns
        .iter()
        .map(|v| {
            json!({
                "uniqueId": v.vuln_id,
                "category": "Dependency-Track Finding",
                "issueName": v.title.clone().unwrap_or_else(|| v.vuln_id.clone()),
                "severity": v.severity.rank() as f64,
                "applicationVersionId": cfg.application_version_id,
                "details": v.description.clone().unwrap_or_default(),
                "cvssScore": v.cvss_v3_base_score,
            })
        })
        .collect();
    json!({
        "applicationVersionId": cfg.application_version_id,
        "items": items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Severity, VulnSource};

    #[test]
    fn payload_carries_app_version() {
        let cfg = FortifyConfig::new("https://ssc", "ci", 99);
        let v = Vulnerability::new("CVE-1", VulnSource::Nvd);
        let p = build_fortify_payload(&cfg, &[v]);
        assert_eq!(p["applicationVersionId"], 99);
        assert_eq!(p["items"][0]["uniqueId"], "CVE-1");
    }

    #[test]
    fn severity_to_numeric_rank() {
        let cfg = FortifyConfig::new("u", "t", 1);
        let mut v = Vulnerability::new("X", VulnSource::Nvd);
        v.severity = Severity::Critical;
        let p = build_fortify_payload(&cfg, &[v]);
        assert_eq!(p["items"][0]["severity"], 5.0);
    }

    #[test]
    fn empty_vulns_empty_items() {
        let cfg = FortifyConfig::new("u", "t", 1);
        let p = build_fortify_payload(&cfg, &[]);
        assert_eq!(p["items"].as_array().unwrap().len(), 0);
    }
}
