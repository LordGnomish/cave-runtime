// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/vulnsrc/vulnsrc.go
//! Feed source parsers.
//!
//! Each source module re-uses [`FeedRecord`] as its on-disk JSON shape — the
//! trivy-db CI builds feed dirs that look like
//! `<source>/<ecosystem>/<package>/<CVE>.json`. We accept those bytes (one
//! file at a time, or stdin-streamed) and emit `(Vulnerability, Vec<Advisory>)`.

pub mod almalinux;
pub mod alpine;
pub mod debian;
pub mod ghsa;
pub mod nvd;
pub mod redhat;

use crate::{Advisory, Result, Severity, Vulnerability};
use serde::{Deserialize, Serialize};

/// Cross-source common feed record shape.
///
/// trivy-db actually has source-specific structs; we collapse to one struct
/// because every source's per-CVE file already contains these fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedRecord {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub references: Vec<String>,
    /// Per-ecosystem package list.
    #[serde(default)]
    pub packages: Vec<FeedPackage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedPackage {
    pub ecosystem: String,
    pub name: String,
    #[serde(default)]
    pub fixed_version: String,
    #[serde(default)]
    pub affected_version: String,
}

impl FeedRecord {
    /// Decompose into one [`Vulnerability`] plus N [`Advisory`] (one per pkg).
    pub fn into_vuln_and_advisories(self) -> (Vulnerability, Vec<Advisory>) {
        let sev = Severity::parse(&self.severity);
        let v = Vulnerability {
            id: self.id.clone(),
            title: self.title,
            description: self.description,
            severity: sev,
            cwe_ids: Vec::new(),
            references: self.references,
            cvss_v3: None,
            published_date: None,
            last_modified_date: None,
        };
        let adv = self
            .packages
            .into_iter()
            .map(|p| Advisory {
                vulnerability_id: self.id.clone(),
                package_name: p.name,
                ecosystem: p.ecosystem,
                fixed_version: p.fixed_version,
                affected_version: p.affected_version,
                severity: sev,
                data_source: "feed".to_string(),
            })
            .collect();
        (v, adv)
    }

    /// Parse one JSON feed file.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }
}
