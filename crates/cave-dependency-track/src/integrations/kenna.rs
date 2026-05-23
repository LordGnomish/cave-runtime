// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kenna Security ("Cisco Vulnerability Management") bridge.
//! Mirrors `integrations.kenna.KennaSecurityUploader`.

use crate::models::{Severity, Vulnerability};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct KennaConfig {
    pub api_token: String,
    pub connector_id: u64,
    pub asset_external_id: String,
}

impl KennaConfig {
    pub fn new(token: impl Into<String>, connector: u64, asset: impl Into<String>) -> Self {
        Self {
            api_token: token.into(),
            connector_id: connector,
            asset_external_id: asset.into(),
        }
    }
}

/// Generates the Kenna Data Importer Json (KDI) document.
pub fn build_kenna_payload(cfg: &KennaConfig, vulns: &[Vulnerability]) -> Value {
    let vulns_block: Vec<Value> = vulns
        .iter()
        .map(|v| {
            json!({
                "scanner_identifier": v.vuln_id,
                "scanner_type": "Dependency-Track",
                "scanner_score": cvss_to_kenna_score(v.cvss_v3_base_score, v.severity),
                "cve_identifiers": vec![v.vuln_id.clone()],
                "details": v.description.clone().unwrap_or_default(),
            })
        })
        .collect();
    json!({
        "skip_autoclose": false,
        "assets": [{
            "external_id": cfg.asset_external_id,
            "tags": ["cave-runtime"],
            "vulns": vulns_block,
        }],
        "connector_id": cfg.connector_id,
    })
}

fn cvss_to_kenna_score(score: Option<f64>, sev: Severity) -> u8 {
    if let Some(s) = score {
        return (s.clamp(0.0, 10.0)) as u8;
    }
    match sev {
        Severity::Critical => 10,
        Severity::High => 8,
        Severity::Medium => 5,
        Severity::Low => 3,
        Severity::Info => 1,
        Severity::Unassigned => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VulnSource;

    #[test]
    fn payload_attaches_asset_external_id() {
        let cfg = KennaConfig::new("t", 1, "asset-x");
        let v = Vulnerability::new("CVE-1", VulnSource::Nvd);
        let p = build_kenna_payload(&cfg, &[v]);
        assert_eq!(p["assets"][0]["external_id"], "asset-x");
        assert_eq!(p["connector_id"], 1);
    }

    #[test]
    fn cvss_clamped_to_0_10() {
        assert_eq!(cvss_to_kenna_score(Some(11.0), Severity::Critical), 10);
        assert_eq!(cvss_to_kenna_score(Some(-1.0), Severity::Low), 0);
    }

    #[test]
    fn severity_fallback_when_no_cvss() {
        assert_eq!(cvss_to_kenna_score(None, Severity::Critical), 10);
        assert_eq!(cvss_to_kenna_score(None, Severity::Info), 1);
    }

    #[test]
    fn empty_vulns_empty_block() {
        let cfg = KennaConfig::new("t", 1, "a");
        let p = build_kenna_payload(&cfg, &[]);
        assert_eq!(p["assets"][0]["vulns"].as_array().unwrap().len(), 0);
    }
}
