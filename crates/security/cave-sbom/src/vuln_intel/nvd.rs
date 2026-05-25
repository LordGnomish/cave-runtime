// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/nvd/NvdParser.java
//   src/main/java/org/dependencytrack/parser/nvd/api20/ModelConverter.java
//
//! NVD CVE 2.0 JSON parser. Reads NVD `/rest/json/cves/2.0` payloads.

use crate::models::{AffectedRange, AnalysisState, Severity, VulnIntel, VulnSource};
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum NvdError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct NvdResponse {
    #[serde(default)]
    vulnerabilities: Vec<NvdVulnEnvelope>,
}

#[derive(Debug, Deserialize)]
struct NvdVulnEnvelope {
    cve: NvdCve,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NvdCve {
    id: String,
    #[serde(default)]
    descriptions: Vec<NvdDesc>,
    #[serde(default)]
    metrics: NvdMetrics,
    #[serde(default)]
    weaknesses: Vec<NvdWeakness>,
    #[serde(default)]
    references: Vec<NvdRef>,
    #[serde(default)]
    configurations: Vec<NvdConfig>,
    published: Option<String>,
    last_modified: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NvdDesc {
    lang: String,
    value: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NvdMetrics {
    #[serde(default)]
    cvss_metric_v31: Vec<NvdCvssV3Entry>,
    #[serde(default)]
    cvss_metric_v30: Vec<NvdCvssV3Entry>,
    #[serde(default)]
    cvss_metric_v2: Vec<NvdCvssV2Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NvdCvssV3Entry {
    cvss_data: NvdCvssV3Data,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NvdCvssV3Data {
    base_score: f32,
    vector_string: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NvdCvssV2Entry {
    cvss_data: NvdCvssV2Data,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NvdCvssV2Data {
    base_score: f32,
}

#[derive(Debug, Deserialize)]
struct NvdWeakness {
    #[serde(default)]
    description: Vec<NvdDesc>,
}

#[derive(Debug, Deserialize)]
struct NvdRef {
    url: String,
}

#[derive(Debug, Deserialize)]
struct NvdConfig {
    #[serde(default)]
    nodes: Vec<NvdNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NvdNode {
    #[serde(default)]
    cpe_match: Vec<NvdCpeMatch>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NvdCpeMatch {
    criteria: String,
    #[serde(default)]
    version_start_including: Option<String>,
    #[serde(default)]
    version_end_excluding: Option<String>,
}

/// Parse an NVD `/rest/json/cves/2.0` response into normalized advisories.
pub fn parse_cves_response(input: &[u8]) -> Result<Vec<VulnIntel>, NvdError> {
    let r: NvdResponse = serde_json::from_slice(input)?;
    Ok(r.vulnerabilities
        .into_iter()
        .map(envelope_to_intel)
        .collect())
}

fn envelope_to_intel(env: NvdVulnEnvelope) -> VulnIntel {
    let cve = env.cve;
    let description = cve
        .descriptions
        .iter()
        .find(|d| d.lang == "en")
        .map(|d| d.value.clone())
        .unwrap_or_default();
    let v31 = cve.metrics.cvss_metric_v31.first();
    let v30 = cve.metrics.cvss_metric_v30.first();
    let v3 = v31.or(v30);
    let cvss_v3_base = v3.map(|m| m.cvss_data.base_score);
    let cvss_v3_vector = v3.and_then(|m| m.cvss_data.vector_string.clone());
    let cvss_v2_base = cve
        .metrics
        .cvss_metric_v2
        .first()
        .map(|m| m.cvss_data.base_score);
    let severity = cvss_v3_base
        .map(Severity::from_cvss_v3)
        .unwrap_or(Severity::Unassigned);
    let cwes = cve
        .weaknesses
        .iter()
        .flat_map(|w| w.description.iter())
        .filter_map(|d| {
            d.value
                .strip_prefix("CWE-")
                .and_then(|n| n.parse::<u32>().ok())
        })
        .collect();
    let references: Vec<String> = cve.references.iter().map(|r| r.url.clone()).collect();
    let mut affected: Vec<AffectedRange> = Vec::new();
    for c in &cve.configurations {
        for n in &c.nodes {
            for m in &n.cpe_match {
                let (vendor, product) = parse_cpe_vendor_product(&m.criteria);
                let lower = m
                    .version_start_including
                    .clone()
                    .map(|v| format!(">={}", v))
                    .unwrap_or_default();
                let upper = m
                    .version_end_excluding
                    .clone()
                    .map(|v| format!("<{}", v))
                    .unwrap_or_default();
                let vers = match (lower.is_empty(), upper.is_empty()) {
                    (true, true) => "*".to_string(),
                    (true, false) => upper,
                    (false, true) => lower,
                    (false, false) => format!("{} {}", lower, upper),
                };
                affected.push(AffectedRange {
                    purl_type: "cpe".into(),
                    namespace: vendor,
                    name: product,
                    vers,
                    fixed: m.version_end_excluding.clone(),
                });
            }
        }
    }
    VulnIntel {
        id: Uuid::new_v4(),
        vuln_id: cve.id,
        source: VulnSource::Nvd,
        title: String::new(),
        description,
        severity,
        cvss_v3_base,
        cvss_v3_vector,
        cvss_v2_base,
        epss_score: None,
        epss_percentile: None,
        cwes,
        references,
        affected,
        published: parse_dt(cve.published.as_deref()),
        modified: parse_dt(cve.last_modified.as_deref()),
        state: AnalysisState::NotSet,
    }
}

fn parse_dt(s: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    s.and_then(|s| {
        // NVD timestamps are ISO 8601 without timezone, e.g. "2024-01-30T20:00:00.000"
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.with_timezone(&chrono::Utc))
            .or_else(|| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                    .ok()
                    .map(|d| chrono::TimeZone::from_utc_datetime(&chrono::Utc, &d))
            })
    })
}

/// Parse CPE 2.3 URI to extract (vendor, product) tuple.
/// Spec: NIST IR 7695 §6 "URI binding".
pub fn parse_cpe_vendor_product(cpe: &str) -> (Option<String>, String) {
    let parts: Vec<&str> = cpe.split(':').collect();
    // cpe:2.3:a:vendor:product:version:...
    if parts.len() >= 5 && parts[0] == "cpe" {
        let vendor = parts
            .get(3)
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let product = parts.get(4).map(|s| s.to_string()).unwrap_or_default();
        (vendor, product)
    } else {
        (None, cpe.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "vulnerabilities": [
        {
          "cve": {
            "id": "CVE-2024-12345",
            "descriptions": [{ "lang":"en", "value":"Buffer overflow in foo." }],
            "metrics": {
              "cvssMetricV31": [{
                "cvssData": {
                  "baseScore": 9.8,
                  "vectorString": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"
                }
              }]
            },
            "weaknesses": [{ "description":[{"lang":"en","value":"CWE-119"}]}],
            "references": [{ "url":"https://nvd.nist.gov/vuln/detail/CVE-2024-12345" }],
            "configurations": [{
              "nodes": [{
                "cpeMatch": [{
                  "criteria": "cpe:2.3:a:acme:foo:*:*:*:*:*:*:*:*",
                  "versionStartIncluding": "1.0.0",
                  "versionEndExcluding": "1.2.3"
                }]
              }]
            }],
            "published": "2024-01-30T20:00:00.000",
            "lastModified": "2024-02-01T12:00:00.000"
          }
        }
      ]
    }"#;

    #[test]
    fn parse_extracts_advisory() {
        let v = parse_cves_response(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].vuln_id, "CVE-2024-12345");
        assert_eq!(v[0].source, VulnSource::Nvd);
        assert_eq!(v[0].cvss_v3_base, Some(9.8));
        assert_eq!(v[0].severity, Severity::Critical);
        assert_eq!(v[0].cwes, vec![119]);
    }

    #[test]
    fn parse_extracts_affected_cpe_range() {
        let v = parse_cves_response(SAMPLE.as_bytes()).unwrap();
        let af = &v[0].affected;
        assert_eq!(af.len(), 1);
        assert_eq!(af[0].purl_type, "cpe");
        assert_eq!(af[0].namespace.as_deref(), Some("acme"));
        assert_eq!(af[0].name, "foo");
        assert_eq!(af[0].vers, ">=1.0.0 <1.2.3");
        assert_eq!(af[0].fixed.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn parse_empty_response() {
        let v = parse_cves_response(br#"{"vulnerabilities":[]}"#).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn parse_cpe_v23() {
        let (v, p) = parse_cpe_vendor_product("cpe:2.3:a:apache:log4j:2.14.0:*:*:*:*:*:*:*");
        assert_eq!(v.as_deref(), Some("apache"));
        assert_eq!(p, "log4j");
    }

    #[test]
    fn parse_dt_iso_no_tz() {
        let d = parse_dt(Some("2024-01-30T20:00:00.000"));
        assert!(d.is_some());
    }

    #[test]
    fn parse_cve_with_no_cvss_v3_falls_back_unassigned() {
        let blob = br#"{"vulnerabilities":[{"cve":{
            "id":"CVE-2020-1","descriptions":[{"lang":"en","value":"x"}],
            "metrics":{},"weaknesses":[],"references":[],"configurations":[]
        }}]}"#;
        let v = parse_cves_response(blob).unwrap();
        assert_eq!(v[0].severity, Severity::Unassigned);
    }
}
