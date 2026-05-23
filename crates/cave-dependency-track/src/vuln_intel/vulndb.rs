// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! VulnDB API response parser.  Mirrors `parser.vulndb.VulnDbParser`.

use crate::error::{Error, Result};
use crate::models::{Severity, VulnSource, Vulnerability};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct VulnDbEntry {
    pub vulndb_id: u64,
    pub title: String,
    pub description: Option<String>,
    pub severity: Severity,
    pub cvss_v3_score: Option<f64>,
    pub cve_ids: Vec<String>,
}

#[derive(Deserialize)]
struct RawWrap {
    #[serde(default)]
    results: Vec<RawEntry>,
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(rename = "vulndb_id", default)]
    vulndb_id: Option<u64>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(rename = "cvss_v3_metrics", default)]
    cvss_v3_metrics: Vec<RawCvss>,
    #[serde(default, rename = "ext_references")]
    ext_references: Vec<RawExtRef>,
}

#[derive(Deserialize)]
struct RawCvss {
    #[serde(default)]
    score: Option<f64>,
}

#[derive(Deserialize)]
struct RawExtRef {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    value: String,
}

pub fn parse_vulndb_response(input: &str) -> Result<Vec<VulnDbEntry>> {
    let raw: RawWrap =
        serde_json::from_str(input).map_err(|e| Error::Parse(format!("vulndb: {}", e)))?;
    let mut out = Vec::with_capacity(raw.results.len());
    for r in raw.results {
        let Some(id) = r.vulndb_id else { continue };
        let score = r.cvss_v3_metrics.into_iter().find_map(|m| m.score);
        let severity = score
            .map(Severity::from_cvss_v3)
            .unwrap_or(Severity::Unassigned);
        let cve_ids = r
            .ext_references
            .into_iter()
            .filter(|r| r.r#type.eq_ignore_ascii_case("cve id"))
            .map(|r| r.value)
            .collect();
        out.push(VulnDbEntry {
            vulndb_id: id,
            title: r.title,
            description: r.description,
            severity,
            cvss_v3_score: score,
            cve_ids,
        });
    }
    Ok(out)
}

impl VulnDbEntry {
    pub fn into_vuln(self) -> Vulnerability {
        let id = format!("VulnDB-{}", self.vulndb_id);
        let mut v = Vulnerability::new(id, VulnSource::Vulndb);
        v.title = Some(self.title);
        v.description = self.description;
        v.severity = self.severity;
        v.cvss_v3_base_score = self.cvss_v3_score;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "results":[
        {"vulndb_id":12345,"title":"Sample VulnDB","description":"D",
         "cvss_v3_metrics":[{"score":7.5}],
         "ext_references":[{"type":"CVE ID","value":"CVE-2026-77"}]},
        {"title":"missing id"}
      ]}"#;

    #[test]
    fn parses_results_skipping_missing_id() {
        let v = parse_vulndb_response(SAMPLE).unwrap();
        assert_eq!(v.len(), 1);
        let e = &v[0];
        assert_eq!(e.vulndb_id, 12345);
        assert_eq!(e.cvss_v3_score, Some(7.5));
        assert_eq!(e.cve_ids, vec!["CVE-2026-77"]);
    }

    #[test]
    fn into_vuln_prefixes_id() {
        let v = parse_vulndb_response(SAMPLE).unwrap()[0].clone().into_vuln();
        assert_eq!(v.vuln_id, "VulnDB-12345");
        assert_eq!(v.severity, Severity::High);
    }

    #[test]
    fn malformed_errors() {
        assert!(matches!(parse_vulndb_response("{"), Err(Error::Parse(_))));
    }
}
