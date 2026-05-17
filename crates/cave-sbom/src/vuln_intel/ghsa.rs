// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/github/graphql/GitHubSecurityAdvisoryParser.java
//   docs.github.com/graphql/reference/objects#securityadvisory (spec reference)
//
//! GitHub Security Advisory (GHSA) parser — GraphQL response shape.

use crate::models::{AffectedRange, AnalysisState, Severity, VulnIntel, VulnSource};
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum GhsaError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("missing data block")]
    MissingData,
}

#[derive(Debug, Deserialize)]
struct GhsaResponse {
    data: Option<GhsaData>,
}

#[derive(Debug, Deserialize)]
struct GhsaData {
    #[serde(rename = "securityAdvisories")]
    security_advisories: GhsaConnection,
}

#[derive(Debug, Deserialize)]
struct GhsaConnection {
    #[serde(default)]
    nodes: Vec<GhsaAdvisory>,
}

#[derive(Debug, Deserialize)]
struct GhsaAdvisory {
    #[serde(rename = "ghsaId")]
    ghsa_id: String,
    summary: Option<String>,
    description: Option<String>,
    severity: Option<String>,
    #[serde(default)]
    cwes: GhsaCweConn,
    cvss: Option<GhsaCvss>,
    #[serde(rename = "publishedAt")]
    published_at: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
    #[serde(default)]
    identifiers: Vec<GhsaIdentifier>,
    #[serde(default)]
    references: Vec<GhsaRef>,
    #[serde(default)]
    vulnerabilities: GhsaVulnConn,
}

#[derive(Debug, Default, Deserialize)]
struct GhsaCweConn {
    #[serde(default)]
    nodes: Vec<GhsaCwe>,
}

#[derive(Debug, Deserialize)]
struct GhsaCwe {
    #[serde(rename = "cweId")]
    cwe_id: String,
}

#[derive(Debug, Deserialize)]
struct GhsaCvss {
    score: Option<f32>,
    #[serde(rename = "vectorString")]
    vector: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhsaIdentifier {
    #[serde(rename = "type")]
    kind: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct GhsaRef {
    url: String,
}

#[derive(Debug, Default, Deserialize)]
struct GhsaVulnConn {
    #[serde(default)]
    nodes: Vec<GhsaVulnPkg>,
}

#[derive(Debug, Deserialize)]
struct GhsaVulnPkg {
    #[serde(rename = "vulnerableVersionRange")]
    vulnerable_version_range: Option<String>,
    #[serde(rename = "firstPatchedVersion")]
    first_patched_version: Option<GhsaPatched>,
    package: GhsaPackage,
}

#[derive(Debug, Deserialize)]
struct GhsaPatched {
    identifier: String,
}

#[derive(Debug, Deserialize)]
struct GhsaPackage {
    ecosystem: Option<String>,
    name: String,
}

/// Parse a GraphQL `securityAdvisories` response.
pub fn parse_graphql_response(input: &[u8]) -> Result<Vec<VulnIntel>, GhsaError> {
    let r: GhsaResponse = serde_json::from_slice(input)?;
    let data = r.data.ok_or(GhsaError::MissingData)?;
    Ok(data
        .security_advisories
        .nodes
        .into_iter()
        .map(advisory_to_intel)
        .collect())
}

fn advisory_to_intel(a: GhsaAdvisory) -> VulnIntel {
    // Prefer CVE alias as primary id.
    let primary = a
        .identifiers
        .iter()
        .find(|i| i.kind.eq_ignore_ascii_case("CVE"))
        .map(|i| i.value.clone())
        .unwrap_or_else(|| a.ghsa_id.clone());
    let severity = match a.severity.as_deref().map(str::to_ascii_uppercase).as_deref() {
        Some("CRITICAL") => Severity::Critical,
        Some("HIGH") => Severity::High,
        Some("MODERATE") => Severity::Medium,
        Some("MEDIUM") => Severity::Medium,
        Some("LOW") => Severity::Low,
        _ => Severity::Unassigned,
    };
    let cwes = a
        .cwes
        .nodes
        .iter()
        .filter_map(|c| c.cwe_id.strip_prefix("CWE-").and_then(|n| n.parse().ok()))
        .collect();
    let references = a.references.iter().map(|r| r.url.clone()).collect();
    let affected = a
        .vulnerabilities
        .nodes
        .iter()
        .map(|v| AffectedRange {
            purl_type: v
                .package
                .ecosystem
                .as_deref()
                .map(|e| match e.to_ascii_lowercase().as_str() {
                    "npm" => "npm".into(),
                    "pip" => "pypi".into(),
                    "maven" => "maven".into(),
                    "rubygems" => "gem".into(),
                    "nuget" => "nuget".into(),
                    "composer" => "composer".into(),
                    "go" => "golang".into(),
                    "rust" => "cargo".into(),
                    s => s.to_string(),
                })
                .unwrap_or_else(|| "unknown".into()),
            namespace: None,
            name: v.package.name.clone(),
            vers: v
                .vulnerable_version_range
                .clone()
                .unwrap_or_else(|| "*".into()),
            fixed: v
                .first_patched_version
                .as_ref()
                .map(|p| p.identifier.clone()),
        })
        .collect();
    let cvss_v3_base = a.cvss.as_ref().and_then(|c| c.score);
    let cvss_v3_vector = a.cvss.as_ref().and_then(|c| c.vector.clone());
    VulnIntel {
        id: Uuid::new_v4(),
        vuln_id: primary,
        source: VulnSource::Ghsa,
        title: a.summary.unwrap_or_default(),
        description: a.description.unwrap_or_default(),
        severity,
        cvss_v3_base,
        cvss_v3_vector,
        cvss_v2_base: None,
        epss_score: None,
        epss_percentile: None,
        cwes,
        references,
        affected,
        published: a
            .published_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&chrono::Utc)),
        modified: a
            .updated_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&chrono::Utc)),
        state: AnalysisState::NotSet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "data": {
        "securityAdvisories": {
          "nodes": [
            {
              "ghsaId": "GHSA-jf85-cpcp-j695",
              "summary": "Prototype Pollution in lodash",
              "description": "Versions of lodash prior to 4.17.12...",
              "severity": "HIGH",
              "cwes": { "nodes":[ { "cweId":"CWE-1321" } ] },
              "cvss": { "score": 7.4, "vectorString":"CVSS:3.0/AV:N/..." },
              "publishedAt":"2019-07-19T22:23:00Z",
              "updatedAt":"2024-01-01T00:00:00Z",
              "identifiers":[ {"type":"GHSA","value":"GHSA-jf85-cpcp-j695"},
                              {"type":"CVE","value":"CVE-2019-10744"} ],
              "references":[ {"url":"https://github.com/advisories/GHSA-jf85-cpcp-j695"} ],
              "vulnerabilities": {
                "nodes":[ {
                  "vulnerableVersionRange": "< 4.17.12",
                  "firstPatchedVersion": { "identifier":"4.17.12" },
                  "package": { "ecosystem":"NPM", "name":"lodash" }
                } ]
              }
            }
          ]
        }
      }
    }"#;

    #[test]
    fn parse_extracts_advisory() {
        let v = parse_graphql_response(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].vuln_id, "CVE-2019-10744");
        assert_eq!(v[0].source, VulnSource::Ghsa);
        assert_eq!(v[0].severity, Severity::High);
        assert_eq!(v[0].cvss_v3_base, Some(7.4));
    }

    #[test]
    fn parse_extracts_affected_npm_range() {
        let v = parse_graphql_response(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v[0].affected.len(), 1);
        assert_eq!(v[0].affected[0].purl_type, "npm");
        assert_eq!(v[0].affected[0].name, "lodash");
        assert_eq!(v[0].affected[0].vers, "< 4.17.12");
        assert_eq!(v[0].affected[0].fixed.as_deref(), Some("4.17.12"));
    }

    #[test]
    fn parse_extracts_cwes() {
        let v = parse_graphql_response(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v[0].cwes, vec![1321]);
    }

    #[test]
    fn parse_falls_back_to_ghsa_id_when_no_cve() {
        let blob = r#"{"data":{"securityAdvisories":{"nodes":[{
          "ghsaId":"GHSA-xxxx","summary":"t","description":"d","severity":"LOW",
          "cwes":{"nodes":[]},"cvss":{"score":null},"publishedAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:00:00Z",
          "identifiers":[{"type":"GHSA","value":"GHSA-xxxx"}],"references":[],
          "vulnerabilities":{"nodes":[]}
        }]}}}"#;
        let v = parse_graphql_response(blob.as_bytes()).unwrap();
        assert_eq!(v[0].vuln_id, "GHSA-xxxx");
    }

    #[test]
    fn parse_handles_missing_data_block() {
        let blob = r#"{"errors":[{"message":"oops"}]}"#;
        assert!(matches!(parse_graphql_response(blob.as_bytes()), Err(GhsaError::MissingData)));
    }

    #[test]
    fn severity_moderate_maps_to_medium() {
        let blob = r#"{"data":{"securityAdvisories":{"nodes":[{
          "ghsaId":"GHSA-z","summary":"t","description":"d","severity":"MODERATE",
          "cwes":{"nodes":[]},"cvss":{"score":null},"publishedAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:00:00Z",
          "identifiers":[{"type":"GHSA","value":"GHSA-z"}],"references":[],
          "vulnerabilities":{"nodes":[]}
        }]}}}"#;
        let v = parse_graphql_response(blob.as_bytes()).unwrap();
        assert_eq!(v[0].severity, Severity::Medium);
    }
}
