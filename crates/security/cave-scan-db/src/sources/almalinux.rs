// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/vulnsrc/alma/alma.go
//! AlmaLinux errata feed parser.
//!
//! Alma's errata JSON is one ALSA-bundle per file:
//! ```json
//! {
//!   "id": "ALSA-2024:1234",
//!   "severity": "Important",
//!   "references": [{ "id": "CVE-2024-0001", "type": "cve" }],
//!   "packages": [{ "name": "openssl", "version": "1.1.1k-12.el8_9" }]
//! }
//! ```

use crate::{Advisory, DbError, Result, Severity};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AlmaErrata {
    pub id: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub references: Vec<AlmaRef>,
    #[serde(default)]
    pub packages: Vec<AlmaPkg>,
    #[serde(default = "default_release")]
    pub release: String,
}

fn default_release() -> String {
    "8".to_string()
}

#[derive(Debug, Deserialize)]
pub struct AlmaRef {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct AlmaPkg {
    pub name: String,
    pub version: String,
}

/// Parse one Alma errata record → advisory list (one per CVE × per pkg).
pub fn parse(bytes: &[u8]) -> Result<Vec<Advisory>> {
    let e: AlmaErrata =
        serde_json::from_slice(bytes).map_err(|e| DbError::InvalidFeed(e.to_string()))?;
    let sev = Severity::parse(&e.severity);
    let eco = format!("alma:{}", e.release);
    let cves: Vec<String> = e
        .references
        .into_iter()
        .filter(|r| r.kind == "cve")
        .map(|r| r.id)
        .collect();
    let mut out = Vec::new();
    for p in e.packages {
        for cve in &cves {
            out.push(Advisory {
                vulnerability_id: cve.clone(),
                package_name: p.name.clone(),
                ecosystem: eco.clone(),
                fixed_version: p.version.clone(),
                affected_version: String::new(),
                severity: sev,
                data_source: "almalinux".into(),
            });
        }
    }
    Ok(out)
}
