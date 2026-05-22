// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/vulnsrc/debian/debian.go
//! Debian Security Tracker feed parser.
//!
//! Schema modelled on Debian's `tracker.debian.org` JSON export:
//! `{ "<pkg>": { "<cve>": { "scope": "...", "releases": { "<codename>": { "status": "...", "fixed_version": "..." } } } } }`

use crate::{Advisory, DbError, Result, Severity};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub struct DebianBundle(pub BTreeMap<String, BTreeMap<String, DebianEntry>>);

#[derive(Debug, Deserialize)]
pub struct DebianEntry {
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub description: String,
    pub releases: BTreeMap<String, DebianRelease>,
}

#[derive(Debug, Deserialize)]
pub struct DebianRelease {
    pub status: String,
    #[serde(default)]
    pub fixed_version: String,
    #[serde(default)]
    pub urgency: String,
}

/// Parse the Debian tracker JSON → flat advisory list.
pub fn parse(bytes: &[u8]) -> Result<Vec<Advisory>> {
    let b: DebianBundle =
        serde_json::from_slice(bytes).map_err(|e| DbError::InvalidFeed(e.to_string()))?;
    let mut out = Vec::new();
    for (pkg, cves) in b.0 {
        for (cve, entry) in cves {
            for (release, info) in entry.releases {
                if info.status == "resolved" || info.status == "open" {
                    out.push(Advisory {
                        vulnerability_id: cve.clone(),
                        package_name: pkg.clone(),
                        ecosystem: format!("debian:{release}"),
                        fixed_version: info.fixed_version,
                        affected_version: String::new(),
                        severity: Severity::parse(&info.urgency),
                        data_source: "debian".into(),
                    });
                }
            }
        }
    }
    Ok(out)
}
