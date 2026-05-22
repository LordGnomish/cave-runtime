// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/vulnsrc/ghsa/ghsa.go
//! GitHub Security Advisories feed parser.
//!
//! GHSA delivers per-language ecosystem advisories. Schema modelled on the
//! `osv.dev` export of GHSA (GHSA → OSV is a 1:1 mapping).

use crate::{Advisory, DbError, Result, Severity, Vulnerability};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GhsaRecord {
    pub id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub severity: Vec<GhsaSeverity>,
    pub affected: Vec<GhsaAffected>,
    #[serde(default)]
    pub references: Vec<GhsaRef>,
}

#[derive(Debug, Deserialize)]
pub struct GhsaSeverity {
    #[serde(rename = "type")]
    pub kind: String,
    pub score: String,
}

#[derive(Debug, Deserialize)]
pub struct GhsaAffected {
    pub package: GhsaPackage,
    #[serde(default)]
    pub ranges: Vec<GhsaRange>,
    #[serde(default)]
    pub database_specific: Option<GhsaDbSpecific>,
}

#[derive(Debug, Deserialize)]
pub struct GhsaPackage {
    pub ecosystem: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct GhsaRange {
    #[serde(rename = "type")]
    pub kind: String,
    pub events: Vec<GhsaEvent>,
}

#[derive(Debug, Deserialize)]
pub struct GhsaEvent {
    #[serde(default)]
    pub introduced: Option<String>,
    #[serde(default)]
    pub fixed: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GhsaDbSpecific {
    #[serde(default)]
    pub severity: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GhsaRef {
    pub url: String,
}

/// Parse one OSV/GHSA JSON file → vulnerability + per-package advisories.
pub fn parse(bytes: &[u8]) -> Result<(Vulnerability, Vec<Advisory>)> {
    let r: GhsaRecord =
        serde_json::from_slice(bytes).map_err(|e| DbError::InvalidFeed(e.to_string()))?;
    let sev_str = r
        .affected
        .iter()
        .find_map(|a| {
            a.database_specific
                .as_ref()
                .and_then(|d| d.severity.clone())
        })
        .unwrap_or_else(|| {
            r.severity
                .iter()
                .find(|s| s.kind == "CVSS_V3")
                .map(|s| s.score.clone())
                .unwrap_or_default()
        });
    let sev = Severity::parse(&sev_str);
    let v = Vulnerability {
        id: r.id.clone(),
        title: r.summary,
        description: r.details,
        severity: sev,
        cwe_ids: Vec::new(),
        references: r.references.into_iter().map(|x| x.url).collect(),
        cvss_v3: None,
        published_date: None,
        last_modified_date: None,
    };
    let mut adv = Vec::new();
    for a in r.affected {
        let (intro, fix) = collapse_range(&a.ranges);
        let aff_spec = if intro.is_empty() {
            String::new()
        } else if fix.is_empty() {
            format!(">={intro}")
        } else {
            format!(">={intro},<{fix}")
        };
        adv.push(Advisory {
            vulnerability_id: r.id.clone(),
            package_name: a.package.name,
            ecosystem: a.package.ecosystem,
            fixed_version: fix,
            affected_version: aff_spec,
            severity: sev,
            data_source: "ghsa".into(),
        });
    }
    Ok((v, adv))
}

fn collapse_range(rs: &[GhsaRange]) -> (String, String) {
    let mut intro = String::new();
    let mut fix = String::new();
    for r in rs {
        for e in &r.events {
            if let Some(i) = &e.introduced {
                if intro.is_empty() {
                    intro = i.clone();
                }
            }
            if let Some(f) = &e.fixed {
                fix = f.clone();
            }
        }
    }
    (intro, fix)
}
