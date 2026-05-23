// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NVD CVE 2.0 JSON parser.  Mirrors `parser.nvd.api20.NvdApi20Parser`.

use crate::error::{Error, Result};
use crate::models::{Severity, VulnSource, Vulnerability};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct NvdCve {
    pub cve_id: String,
    pub description: Option<String>,
    pub cvss_v3_score: Option<f64>,
    pub cvss_v3_vector: Option<String>,
    pub cwes: Vec<u32>,
    pub published: Option<DateTime<Utc>>,
    pub last_modified: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct RawWrap {
    #[serde(default)]
    vulnerabilities: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawItem {
    #[serde(default)]
    cve: RawCve,
}

#[derive(Default, Deserialize)]
struct RawCve {
    #[serde(default)]
    id: String,
    #[serde(default)]
    published: Option<String>,
    #[serde(rename = "lastModified", default)]
    last_modified: Option<String>,
    #[serde(default)]
    descriptions: Vec<RawDesc>,
    #[serde(default)]
    weaknesses: Vec<RawWeakness>,
    #[serde(default)]
    metrics: Option<RawMetrics>,
}

#[derive(Deserialize)]
struct RawDesc {
    #[serde(default)]
    lang: String,
    #[serde(default)]
    value: String,
}

#[derive(Deserialize)]
struct RawWeakness {
    #[serde(default)]
    description: Vec<RawDesc>,
}

#[derive(Deserialize)]
struct RawMetrics {
    #[serde(rename = "cvssMetricV31", default)]
    cvss_v31: Vec<RawCvss>,
    #[serde(rename = "cvssMetricV30", default)]
    cvss_v30: Vec<RawCvss>,
}

#[derive(Deserialize)]
struct RawCvss {
    #[serde(rename = "cvssData", default)]
    cvss_data: Option<RawCvssData>,
}

#[derive(Default, Deserialize)]
struct RawCvssData {
    #[serde(rename = "baseScore", default)]
    base_score: Option<f64>,
    #[serde(rename = "vectorString", default)]
    vector_string: Option<String>,
}

#[cfg(test)]
mod usage_marker {
    // RawCvssData is constructed by serde via `#[derive(Deserialize)]`; the
    // `Default` impl is required for the `pick_cvss` fallback path.
    use super::RawCvssData;
    #[allow(dead_code)]
    fn _ensure_default() -> RawCvssData {
        RawCvssData::default()
    }
}

pub fn parse_nvd_2_0(input: &str) -> Result<Vec<NvdCve>> {
    let wrap: RawWrap =
        serde_json::from_str(input).map_err(|e| Error::Parse(format!("nvd: {}", e)))?;
    let mut out = Vec::with_capacity(wrap.vulnerabilities.len());
    for item in wrap.vulnerabilities {
        let cve = item.cve;
        if cve.id.is_empty() {
            continue;
        }
        let desc = cve
            .descriptions
            .into_iter()
            .find(|d| d.lang.eq_ignore_ascii_case("en"))
            .map(|d| d.value);
        let cwes: Vec<u32> = cve
            .weaknesses
            .into_iter()
            .flat_map(|w| w.description.into_iter())
            .filter_map(|d| parse_cwe(&d.value))
            .collect();
        let (score, vector) = pick_cvss(cve.metrics);
        out.push(NvdCve {
            cve_id: cve.id,
            description: desc,
            cvss_v3_score: score,
            cvss_v3_vector: vector,
            cwes,
            published: cve.published.as_deref().and_then(parse_ts),
            last_modified: cve.last_modified.as_deref().and_then(parse_ts),
        });
    }
    Ok(out)
}

fn pick_cvss(m: Option<RawMetrics>) -> (Option<f64>, Option<String>) {
    let Some(metrics) = m else {
        return (None, None);
    };
    for c in metrics.cvss_v31.into_iter().chain(metrics.cvss_v30) {
        if let Some(data) = c.cvss_data {
            return (data.base_score, data.vector_string);
        }
    }
    (None, None)
}

fn parse_cwe(s: &str) -> Option<u32> {
    s.strip_prefix("CWE-").and_then(|n| n.parse().ok())
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

impl NvdCve {
    pub fn into_vuln(self) -> Vulnerability {
        let severity = self
            .cvss_v3_score
            .map(Severity::from_cvss_v3)
            .unwrap_or(Severity::Unassigned);
        let mut v = Vulnerability::new(self.cve_id, VulnSource::Nvd);
        v.description = self.description;
        v.severity = severity;
        v.cvss_v3_base_score = self.cvss_v3_score;
        v.cvss_v3_vector = self.cvss_v3_vector;
        v.cwes = self.cwes;
        v.published = self.published;
        v.updated = self.last_modified;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "vulnerabilities":[
        {"cve":{
          "id":"CVE-2026-12345",
          "published":"2026-04-01T12:00:00.000Z",
          "lastModified":"2026-04-02T08:00:00.000Z",
          "descriptions":[{"lang":"en","value":"Sample CVE"},{"lang":"es","value":"x"}],
          "weaknesses":[{"description":[{"lang":"en","value":"CWE-79"}]}],
          "metrics":{
            "cvssMetricV31":[{"cvssData":{"baseScore":7.5,"vectorString":"CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N"}}]
          }
        }},
        {"cve":{"id":"","descriptions":[]}}
      ]}"#;

    #[test]
    fn parses_one_cve_skips_empty_id() {
        let cves = parse_nvd_2_0(SAMPLE).unwrap();
        assert_eq!(cves.len(), 1);
        let c = &cves[0];
        assert_eq!(c.cve_id, "CVE-2026-12345");
        assert_eq!(c.cvss_v3_score, Some(7.5));
        assert_eq!(c.cwes, vec![79]);
        assert_eq!(c.description.as_deref(), Some("Sample CVE"));
        assert!(c.published.is_some());
    }

    #[test]
    fn into_vuln_maps_severity_high() {
        let cves = parse_nvd_2_0(SAMPLE).unwrap();
        let v = cves[0].clone().into_vuln();
        assert_eq!(v.severity, Severity::High);
        assert_eq!(v.source, VulnSource::Nvd);
    }

    #[test]
    fn empty_metrics_unassigned() {
        let raw = r#"{"vulnerabilities":[{"cve":{"id":"CVE-X","descriptions":[]}}]}"#;
        let v = parse_nvd_2_0(raw).unwrap()[0].clone().into_vuln();
        assert_eq!(v.severity, Severity::Unassigned);
        assert!(v.cvss_v3_base_score.is_none());
    }

    #[test]
    fn malformed_errors() {
        assert!(matches!(parse_nvd_2_0("{"), Err(Error::Parse(_))));
    }
}
