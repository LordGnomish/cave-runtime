// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Snyk advisory parser (license-permitting subset).
//! Mirrors `parser.snyk.SnykParser`.

use crate::error::{Error, Result};
use crate::models::{Severity, VulnSource, Vulnerability};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct SnykAdvisory {
    pub id: String,
    pub title: String,
    pub severity: Severity,
    pub cvss_score: Option<f64>,
    pub package_name: Option<String>,
    pub identifiers_cve: Vec<String>,
    pub identifiers_ghsa: Vec<String>,
}

#[derive(Deserialize)]
struct RawSnyk {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    severity: Option<String>,
    #[serde(rename = "cvssScore", default)]
    cvss_score: Option<f64>,
    #[serde(rename = "packageName", default)]
    package_name: Option<String>,
    #[serde(default)]
    identifiers: Option<RawIds>,
}

#[derive(Deserialize, Default)]
struct RawIds {
    #[serde(default, rename = "CVE")]
    cve: Vec<String>,
    #[serde(default, rename = "GHSA")]
    ghsa: Vec<String>,
}

pub fn parse_snyk_json(input: &str) -> Result<SnykAdvisory> {
    let raw: RawSnyk =
        serde_json::from_str(input).map_err(|e| Error::Parse(format!("snyk: {}", e)))?;
    if raw.id.is_empty() {
        return Err(Error::Parse("snyk: missing id".into()));
    }
    let severity = match raw.severity.as_deref().unwrap_or("").to_ascii_lowercase().as_str() {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => raw
            .cvss_score
            .map(Severity::from_cvss_v3)
            .unwrap_or(Severity::Unassigned),
    };
    let ids = raw.identifiers.unwrap_or_default();
    Ok(SnykAdvisory {
        id: raw.id,
        title: raw.title,
        severity,
        cvss_score: raw.cvss_score,
        package_name: raw.package_name,
        identifiers_cve: ids.cve,
        identifiers_ghsa: ids.ghsa,
    })
}

impl SnykAdvisory {
    pub fn into_vuln(self) -> Vulnerability {
        let mut v = Vulnerability::new(self.id, VulnSource::Snyk);
        v.title = Some(self.title);
        v.severity = self.severity;
        v.cvss_v3_base_score = self.cvss_score;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "id":"SNYK-JS-LODASH-1018905",
      "title":"Prototype Pollution",
      "severity":"high",
      "cvssScore":7.4,
      "packageName":"lodash",
      "identifiers":{"CVE":["CVE-2020-8203"],"GHSA":["GHSA-p6mc-m468-83gw"]}
    }"#;

    #[test]
    fn parses_full_advisory() {
        let a = parse_snyk_json(SAMPLE).unwrap();
        assert_eq!(a.id, "SNYK-JS-LODASH-1018905");
        assert_eq!(a.severity, Severity::High);
        assert_eq!(a.identifiers_cve, vec!["CVE-2020-8203"]);
        assert_eq!(a.identifiers_ghsa, vec!["GHSA-p6mc-m468-83gw"]);
    }

    #[test]
    fn severity_from_cvss_when_text_missing() {
        let raw = r#"{"id":"S1","title":"t","cvssScore":4.5}"#;
        let v = parse_snyk_json(raw).unwrap().into_vuln();
        assert_eq!(v.severity, Severity::Medium);
    }

    #[test]
    fn missing_id_errors() {
        assert!(matches!(parse_snyk_json("{}"), Err(Error::Parse(_))));
    }

    #[test]
    fn unknown_severity_unassigned_no_cvss() {
        let raw = r#"{"id":"S2","title":"t"}"#;
        let v = parse_snyk_json(raw).unwrap().into_vuln();
        assert_eq!(v.severity, Severity::Unassigned);
    }
}
