// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OSV.dev advisory parser.  Mirrors `parser.osv.OsvAdvisoryParser`.

use crate::error::{Error, Result};
use crate::models::{Severity, VulnSource, Vulnerability};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct OsvAdvisory {
    pub id: String,
    pub summary: Option<String>,
    pub details: Option<String>,
    pub aliases: Vec<String>,
    pub severity_cvss: Option<f64>,
    pub affected_packages: Vec<String>,
}

#[derive(Deserialize)]
struct RawOsv {
    #[serde(default)]
    id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    severity: Vec<RawSev>,
    #[serde(default)]
    affected: Vec<RawAffected>,
}

#[derive(Deserialize)]
struct RawSev {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    score: String,
}

#[derive(Deserialize)]
struct RawAffected {
    #[serde(default)]
    package: Option<RawPkg>,
}

#[derive(Deserialize)]
struct RawPkg {
    #[serde(default)]
    ecosystem: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    purl: Option<String>,
}

pub fn parse_osv_json(input: &str) -> Result<OsvAdvisory> {
    let raw: RawOsv =
        serde_json::from_str(input).map_err(|e| Error::Parse(format!("osv: {}", e)))?;
    if raw.id.is_empty() {
        return Err(Error::Parse("osv: missing id".into()));
    }
    let severity_cvss = raw
        .severity
        .into_iter()
        .find(|s| s.r#type.starts_with("CVSS_V3"))
        .and_then(|s| {
            s.score
                .split('/')
                .next()
                .and_then(|n| n.parse::<f64>().ok())
        });
    let affected_packages = raw
        .affected
        .into_iter()
        .filter_map(|a| a.package)
        .map(|p| {
            p.purl
                .unwrap_or_else(|| format!("{}:{}", p.ecosystem, p.name))
        })
        .collect();
    Ok(OsvAdvisory {
        id: raw.id,
        summary: raw.summary,
        details: raw.details,
        aliases: raw.aliases,
        severity_cvss,
        affected_packages,
    })
}

impl OsvAdvisory {
    pub fn into_vuln(self) -> Vulnerability {
        let severity = self
            .severity_cvss
            .map(Severity::from_cvss_v3)
            .unwrap_or(Severity::Unassigned);
        let mut v = Vulnerability::new(self.id, VulnSource::Osv);
        v.description = self.details.or(self.summary);
        v.severity = severity;
        v.cvss_v3_base_score = self.severity_cvss;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "id":"GHSA-1234-5678-9999",
      "summary":"Sample",
      "details":"Long details",
      "aliases":["CVE-2026-9999"],
      "severity":[{"type":"CVSS_V3","score":"8.1/CVSS:3.1/AV:N"}],
      "affected":[{"package":{"ecosystem":"PyPI","name":"requests","purl":"pkg:pypi/requests"}}]
    }"#;

    #[test]
    fn parses_full_record() {
        let a = parse_osv_json(SAMPLE).unwrap();
        assert_eq!(a.id, "GHSA-1234-5678-9999");
        assert_eq!(a.severity_cvss, Some(8.1));
        assert_eq!(a.aliases, vec!["CVE-2026-9999"]);
        assert_eq!(a.affected_packages, vec!["pkg:pypi/requests"]);
    }

    #[test]
    fn missing_id_errors() {
        assert!(matches!(parse_osv_json(r#"{}"#), Err(Error::Parse(_))));
    }

    #[test]
    fn into_vuln_high_severity() {
        let v = parse_osv_json(SAMPLE).unwrap().into_vuln();
        assert_eq!(v.severity, Severity::High);
        assert_eq!(v.source, VulnSource::Osv);
    }

    #[test]
    fn no_severity_falls_back_unassigned() {
        let raw = r#"{"id":"OSV-1"}"#;
        let v = parse_osv_json(raw).unwrap().into_vuln();
        assert_eq!(v.severity, Severity::Unassigned);
    }
}
