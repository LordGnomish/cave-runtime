// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sonatype OSS Index response parser.
//! Mirrors `parser.ossindex.OssIndexParser`.

use crate::error::{Error, Result};
use crate::models::{Severity, VulnSource, Vulnerability};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct OssIndexReport {
    pub coordinates: String,
    pub vulnerabilities: Vec<OssVuln>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OssVuln {
    pub id: String,
    pub title: String,
    pub cvss_score: Option<f64>,
    pub cvss_vector: Option<String>,
    pub cwe: Option<u32>,
    pub references: Vec<String>,
}

#[derive(Deserialize)]
struct RawWrap {
    #[serde(default)]
    coordinates: String,
    #[serde(default)]
    vulnerabilities: Vec<RawVuln>,
}

#[derive(Deserialize)]
struct RawVuln {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "cvssScore", default)]
    cvss_score: Option<f64>,
    #[serde(rename = "cvssVector", default)]
    cvss_vector: Option<String>,
    #[serde(default)]
    cwe: Option<String>,
    #[serde(default)]
    reference: Option<String>,
    #[serde(default, rename = "externalReferences")]
    external_references: Vec<String>,
}

pub fn parse_ossindex_response(input: &str) -> Result<OssIndexReport> {
    let raw: RawWrap =
        serde_json::from_str(input).map_err(|e| Error::Parse(format!("ossindex: {}", e)))?;
    let vulnerabilities = raw
        .vulnerabilities
        .into_iter()
        .map(|v| {
            let mut refs = v.external_references;
            if let Some(r) = v.reference {
                refs.insert(0, r);
            }
            OssVuln {
                id: v.id,
                title: v.title,
                cvss_score: v.cvss_score,
                cvss_vector: v.cvss_vector,
                cwe: v
                    .cwe
                    .as_deref()
                    .and_then(|s| s.strip_prefix("CWE-"))
                    .and_then(|s| s.parse().ok()),
                references: refs,
            }
        })
        .collect();
    Ok(OssIndexReport {
        coordinates: raw.coordinates,
        vulnerabilities,
    })
}

impl OssVuln {
    pub fn into_vuln(self) -> Vulnerability {
        let mut v = Vulnerability::new(self.id, VulnSource::Ossindex);
        v.title = Some(self.title);
        v.severity = self
            .cvss_score
            .map(Severity::from_cvss_v3)
            .unwrap_or(Severity::Unassigned);
        v.cvss_v3_base_score = self.cvss_score;
        v.cvss_v3_vector = self.cvss_vector;
        if let Some(c) = self.cwe {
            v.cwes = vec![c];
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "coordinates":"pkg:npm/example@1.0.0",
      "vulnerabilities":[{
        "id":"f57bc8d2-ee43-4f12-94e0-1234",
        "title":"Sample",
        "cvssScore":9.8,
        "cvssVector":"CVSS:3.1/...",
        "cwe":"CWE-79",
        "reference":"https://ossindex.sonatype.org/vuln/abc",
        "externalReferences":["https://nvd.nist.gov/vuln/detail/CVE-x"]
      }]
    }"#;

    #[test]
    fn parses_oss_report() {
        let r = parse_ossindex_response(SAMPLE).unwrap();
        assert_eq!(r.coordinates, "pkg:npm/example@1.0.0");
        assert_eq!(r.vulnerabilities.len(), 1);
        let v = &r.vulnerabilities[0];
        assert_eq!(v.cwe, Some(79));
        assert_eq!(v.references.len(), 2);
    }

    #[test]
    fn into_vuln_critical_score() {
        let v = parse_ossindex_response(SAMPLE).unwrap().vulnerabilities[0]
            .clone()
            .into_vuln();
        assert_eq!(v.severity, Severity::Critical);
    }

    #[test]
    fn malformed_errors() {
        assert!(matches!(parse_ossindex_response("{"), Err(Error::Parse(_))));
    }
}
