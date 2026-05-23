// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! OSV schema 1.6.x parser.
//!
//! Mirrors trivy's `pkg/vulnsrc/osv` decoding path for the subset cave-trivy
//! needs: id, aliases, affected[].package.name/ecosystem, affected[]
//! .ranges[].events[].introduced/fixed, severity, summary, references. We
//! ingest the JSON form; in-memory it becomes an `OsvAdvisory` consumed by
//! `vulndb::VulnDb::ingest_osv`.

use crate::error::{TrivyError, TrivyResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OsvAdvisory {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub severity: Vec<OsvSeverity>,
    #[serde(default)]
    pub affected: Vec<OsvAffected>,
    #[serde(default)]
    pub references: Vec<OsvReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsvSeverity {
    #[serde(rename = "type")]
    pub kind: String,
    pub score: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OsvAffected {
    pub package: OsvPackage,
    #[serde(default)]
    pub ranges: Vec<OsvRange>,
    #[serde(default)]
    pub versions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OsvPackage {
    pub ecosystem: String,
    pub name: String,
    #[serde(default)]
    pub purl: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsvRange {
    #[serde(rename = "type")]
    pub kind: String,
    pub events: Vec<OsvEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsvEvent {
    #[serde(default)]
    pub introduced: Option<String>,
    #[serde(default)]
    pub fixed: Option<String>,
    #[serde(default)]
    pub last_affected: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsvReference {
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
}

impl OsvAdvisory {
    pub fn parse(json: &str) -> TrivyResult<Self> {
        serde_json::from_str(json).map_err(|e| TrivyError::parse(format!("osv: {}", e)))
    }

    pub fn parse_batch(json: &str) -> TrivyResult<Vec<Self>> {
        if json.trim_start().starts_with('[') {
            serde_json::from_str(json).map_err(|e| TrivyError::parse(format!("osv batch: {}", e)))
        } else {
            let mut out = Vec::new();
            for line in json.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                out.push(Self::parse(trimmed)?);
            }
            Ok(out)
        }
    }

    /// Heuristic CVSS-derived severity, mirroring trivy's mapping.
    pub fn primary_score_kind(&self) -> Option<&str> {
        self.severity.first().map(|s| s.kind.as_str())
    }

    /// Return all (ecosystem, name, introduced, fixed) tuples flattened.
    pub fn affected_tuples(&self) -> Vec<(String, String, Option<String>, Option<String>)> {
        let mut out = Vec::new();
        for aff in &self.affected {
            for r in &aff.ranges {
                let mut intro: Option<String> = None;
                let mut fixed: Option<String> = None;
                for ev in &r.events {
                    if let Some(i) = &ev.introduced {
                        intro = Some(i.clone());
                    }
                    if let Some(f) = &ev.fixed {
                        fixed = Some(f.clone());
                    }
                }
                out.push((
                    aff.package.ecosystem.clone(),
                    aff.package.name.clone(),
                    intro,
                    fixed,
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "id": "CVE-2026-0001",
        "aliases": ["GHSA-xxxx-yyyy-zzzz"],
        "summary": "demo",
        "severity": [{"type":"CVSS_V3","score":"7.5"}],
        "affected": [
          { "package": { "ecosystem":"npm", "name":"lodash" },
            "ranges": [
              { "type":"SEMVER",
                "events":[{"introduced":"0.0.0"},{"fixed":"4.17.21"}] }
            ]
          }
        ],
        "references":[{"type":"WEB","url":"https://example/CVE-2026-0001"}]
    }"#;

    #[test]
    fn parses_one() {
        let a = OsvAdvisory::parse(SAMPLE).unwrap();
        assert_eq!(a.id, "CVE-2026-0001");
        assert_eq!(a.affected.len(), 1);
        assert_eq!(a.affected[0].package.name, "lodash");
        assert_eq!(a.primary_score_kind(), Some("CVSS_V3"));
        assert_eq!(a.aliases[0], "GHSA-xxxx-yyyy-zzzz");
    }

    #[test]
    fn affected_tuples_flatten() {
        let a = OsvAdvisory::parse(SAMPLE).unwrap();
        let t = a.affected_tuples();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].0, "npm");
        assert_eq!(t[0].1, "lodash");
        assert_eq!(t[0].2.as_deref(), Some("0.0.0"));
        assert_eq!(t[0].3.as_deref(), Some("4.17.21"));
    }

    #[test]
    fn parse_batch_json_array() {
        let batch = format!("[{},{}]", SAMPLE, SAMPLE);
        let v = OsvAdvisory::parse_batch(&batch).unwrap();
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn parse_batch_ndjson() {
        let batch = format!("{}\n{}\n", SAMPLE.replace('\n', " "), SAMPLE.replace('\n', " "));
        let v = OsvAdvisory::parse_batch(&batch).unwrap();
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn parse_rejects_bad_json() {
        assert!(OsvAdvisory::parse("{not json").is_err());
    }
}
