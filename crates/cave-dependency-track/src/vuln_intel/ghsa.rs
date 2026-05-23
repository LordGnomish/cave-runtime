// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GitHub Security Advisory GraphQL response parser.
//! Mirrors `parser.github.GitHubSecurityAdvisoryParser`.

use crate::error::{Error, Result};
use crate::models::{Severity, VulnSource, Vulnerability};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct GhsaAdvisory {
    pub ghsa_id: String,
    pub cve_id: Option<String>,
    pub summary: String,
    pub description: Option<String>,
    pub severity: Severity,
    pub cvss_score: Option<f64>,
    pub cvss_vector: Option<String>,
    pub references: Vec<String>,
    pub cwes: Vec<u32>,
}

#[derive(Deserialize)]
struct RawAdvisory {
    #[serde(rename = "ghsaId", default)]
    ghsa_id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    identifiers: Vec<RawId>,
    #[serde(default)]
    cvss: Option<RawCvss>,
    #[serde(default)]
    cwes: Option<RawCwes>,
    #[serde(default)]
    references: Option<RawRefs>,
}

#[derive(Deserialize)]
struct RawId {
    #[serde(default, rename = "type")]
    id_type: String,
    #[serde(default)]
    value: String,
}

#[derive(Deserialize)]
struct RawCvss {
    #[serde(default)]
    score: Option<f64>,
    #[serde(rename = "vectorString", default)]
    vector_string: Option<String>,
}

#[derive(Deserialize)]
struct RawCwes {
    #[serde(default)]
    nodes: Vec<RawCwe>,
}

#[derive(Deserialize)]
struct RawCwe {
    #[serde(rename = "cweId", default)]
    cwe_id: String,
}

#[derive(Deserialize)]
struct RawRefs {
    #[serde(default)]
    nodes: Vec<RawRef>,
}

#[derive(Deserialize)]
struct RawRef {
    #[serde(default)]
    url: String,
}

pub fn parse_ghsa_json(input: &str) -> Result<GhsaAdvisory> {
    let raw: RawAdvisory =
        serde_json::from_str(input).map_err(|e| Error::Parse(format!("ghsa: {}", e)))?;
    if raw.ghsa_id.is_empty() {
        return Err(Error::Parse("ghsa: missing ghsaId".into()));
    }
    let severity = parse_severity(raw.severity.as_deref().unwrap_or(""));
    let cve_id = raw
        .identifiers
        .iter()
        .find(|i| i.id_type.eq_ignore_ascii_case("CVE"))
        .map(|i| i.value.clone());
    let cwes = raw
        .cwes
        .map(|c| {
            c.nodes
                .into_iter()
                .filter_map(|n| n.cwe_id.strip_prefix("CWE-").and_then(|s| s.parse().ok()))
                .collect()
        })
        .unwrap_or_default();
    let references = raw
        .references
        .map(|r| r.nodes.into_iter().map(|n| n.url).collect())
        .unwrap_or_default();
    let cvss_score = raw.cvss.as_ref().and_then(|c| c.score);
    let cvss_vector = raw.cvss.and_then(|c| c.vector_string);
    Ok(GhsaAdvisory {
        ghsa_id: raw.ghsa_id,
        cve_id,
        summary: raw.summary.unwrap_or_default(),
        description: raw.description,
        severity,
        cvss_score,
        cvss_vector,
        references,
        cwes,
    })
}

fn parse_severity(s: &str) -> Severity {
    match s.to_ascii_uppercase().as_str() {
        "CRITICAL" => Severity::Critical,
        "HIGH" => Severity::High,
        "MODERATE" | "MEDIUM" => Severity::Medium,
        "LOW" => Severity::Low,
        _ => Severity::Unassigned,
    }
}

impl GhsaAdvisory {
    pub fn into_vuln(self) -> Vulnerability {
        let mut v = Vulnerability::new(self.ghsa_id, VulnSource::Github);
        v.title = Some(self.summary);
        v.description = self.description;
        v.severity = self.severity;
        v.cvss_v3_base_score = self.cvss_score;
        v.cvss_v3_vector = self.cvss_vector;
        v.cwes = self.cwes;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "ghsaId":"GHSA-aaaa-bbbb-cccc",
      "summary":"Sample","description":"Big detail",
      "severity":"HIGH",
      "identifiers":[{"type":"GHSA","value":"GHSA-aaaa-bbbb-cccc"},{"type":"CVE","value":"CVE-2026-77"}],
      "cvss":{"score":7.4,"vectorString":"CVSS:3.1/AV:N"},
      "cwes":{"nodes":[{"cweId":"CWE-79"},{"cweId":"CWE-89"}]},
      "references":{"nodes":[{"url":"https://example.com/1"}]}
    }"#;

    #[test]
    fn parses_full_advisory() {
        let g = parse_ghsa_json(SAMPLE).unwrap();
        assert_eq!(g.ghsa_id, "GHSA-aaaa-bbbb-cccc");
        assert_eq!(g.cve_id.as_deref(), Some("CVE-2026-77"));
        assert_eq!(g.severity, Severity::High);
        assert_eq!(g.cwes, vec![79, 89]);
        assert_eq!(g.references, vec!["https://example.com/1"]);
    }

    #[test]
    fn into_vuln_preserves_title_and_score() {
        let v = parse_ghsa_json(SAMPLE).unwrap().into_vuln();
        assert_eq!(v.title.as_deref(), Some("Sample"));
        assert_eq!(v.cvss_v3_base_score, Some(7.4));
    }

    #[test]
    fn unknown_severity_unassigned() {
        let raw = r#"{"ghsaId":"GHSA-x","severity":"unknown"}"#;
        let v = parse_ghsa_json(raw).unwrap().into_vuln();
        assert_eq!(v.severity, Severity::Unassigned);
    }

    #[test]
    fn missing_id_errors() {
        assert!(matches!(parse_ghsa_json("{}"), Err(Error::Parse(_))));
    }
}
