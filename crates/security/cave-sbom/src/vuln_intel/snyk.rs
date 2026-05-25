// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/snyk/SnykParser.java
//
//! Snyk advisory parser — license-permitting subset of Snyk's REST API.
//!
//! Honest scope: Snyk's vulnerability database is commercial and licensed.
//! Dependency-Track's Snyk parser walks the `vulnerabilities` array from a
//! `/v3/orgs/{org}/issues` style response. We mirror only the publicly
//! documented attributes — consumers must supply their own API key.

use crate::models::{AffectedRange, AnalysisState, Severity, VulnIntel, VulnSource};
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum SnykError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct SnykResponse {
    #[serde(default)]
    data: Vec<SnykIssue>,
}

#[derive(Debug, Deserialize)]
struct SnykIssue {
    id: String,
    attributes: SnykAttrs,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct SnykAttrs {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    severities: Vec<SnykSeverity>,
    #[serde(default)]
    problems: Vec<SnykProblem>,
    #[serde(default)]
    coordinates: Vec<SnykCoord>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SnykSeverity {
    level: String,
    score: Option<f32>,
    vector: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SnykProblem {
    #[serde(rename = "type")]
    ptype: String,
    id: String,
}

#[derive(Debug, Deserialize)]
struct SnykCoord {
    #[serde(default)]
    representation: Vec<SnykRep>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SnykRep {
    Purl {
        purl: String,
    },
    Range {
        vulnerable_range: String,
        fixed_in: Option<Vec<String>>,
    },
}

pub fn parse_response(input: &[u8]) -> Result<Vec<VulnIntel>, SnykError> {
    let r: SnykResponse = serde_json::from_slice(input)?;
    Ok(r.data.into_iter().map(issue_to_intel).collect())
}

fn issue_to_intel(issue: SnykIssue) -> VulnIntel {
    let a = issue.attributes;
    // Prefer CVE problem id as primary if present.
    let primary = a
        .problems
        .iter()
        .find(|p| p.ptype.eq_ignore_ascii_case("CVE") || p.id.starts_with("CVE-"))
        .map(|p| p.id.clone())
        .unwrap_or_else(|| issue.id.clone());
    let severity = a
        .severities
        .iter()
        .find(|s| s.score.is_some())
        .map(|s| Severity::from_cvss_v3(s.score.unwrap_or(0.0)))
        .or_else(|| {
            a.severities
                .first()
                .map(|s| match s.level.to_ascii_lowercase().as_str() {
                    "critical" => Severity::Critical,
                    "high" => Severity::High,
                    "medium" => Severity::Medium,
                    "low" => Severity::Low,
                    _ => Severity::Unassigned,
                })
        })
        .unwrap_or(Severity::Unassigned);
    let cvss_v3_base = a.severities.iter().find_map(|s| s.score);
    let cvss_v3_vector = a.severities.iter().find_map(|s| s.vector.clone());
    let cwes = a
        .problems
        .iter()
        .filter_map(|p| {
            p.id.strip_prefix("CWE-")
                .and_then(|n| n.parse::<u32>().ok())
        })
        .collect();
    let mut affected: Vec<AffectedRange> = Vec::new();
    for c in &a.coordinates {
        let mut purl: Option<String> = None;
        let mut vers_range: Option<String> = None;
        let mut fixed: Option<String> = None;
        for rep in &c.representation {
            match rep {
                SnykRep::Purl { purl: p } => purl = Some(p.clone()),
                SnykRep::Range {
                    vulnerable_range,
                    fixed_in,
                } => {
                    vers_range = Some(vulnerable_range.clone());
                    if let Some(fi) = fixed_in {
                        fixed = fi.first().cloned();
                    }
                }
            }
        }
        let (purl_type, name) = purl
            .as_deref()
            .and_then(parse_purl_simple)
            .unwrap_or(("unknown".into(), String::new()));
        affected.push(AffectedRange {
            purl_type,
            namespace: None,
            name,
            vers: vers_range.unwrap_or_else(|| "*".into()),
            fixed,
        });
    }
    VulnIntel {
        id: Uuid::new_v4(),
        vuln_id: primary,
        source: VulnSource::Snyk,
        title: a.title.unwrap_or_default(),
        description: a.description.unwrap_or_default(),
        severity,
        cvss_v3_base,
        cvss_v3_vector,
        cvss_v2_base: None,
        epss_score: None,
        epss_percentile: None,
        cwes,
        references: vec![],
        affected,
        published: a
            .created_at
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

fn parse_purl_simple(purl: &str) -> Option<(String, String)> {
    let rest = purl.strip_prefix("pkg:")?;
    let (ptype, rest2) = rest.split_once('/')?;
    let (name, _ver) = rest2.split_once('@').unwrap_or((rest2, ""));
    let name = name
        .rsplit_once('/')
        .map(|(_, n)| n)
        .unwrap_or(name)
        .to_string();
    Some((ptype.to_string(), name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "data": [
        {
          "id": "SNYK-JS-LODASH-590103",
          "attributes": {
            "title": "Prototype Pollution",
            "description": "lodash before 4.17.12...",
            "severities": [{ "level":"high", "score":7.4, "vector":"CVSS:3.0/..." }],
            "problems": [
              { "type":"CVE", "id":"CVE-2019-10744" },
              { "type":"CWE", "id":"CWE-1321" }
            ],
            "coordinates": [{
              "representation": [
                { "purl":"pkg:npm/lodash" },
                { "vulnerable_range":"<4.17.12", "fixed_in":["4.17.12"] }
              ]
            }],
            "created_at":"2019-07-19T22:23:00Z",
            "updated_at":"2024-01-01T00:00:00Z"
          }
        }
      ]
    }"#;

    #[test]
    fn parse_extracts_advisory() {
        let v = parse_response(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].vuln_id, "CVE-2019-10744");
        assert_eq!(v[0].source, VulnSource::Snyk);
        assert_eq!(v[0].severity, Severity::High);
        assert_eq!(v[0].cvss_v3_base, Some(7.4));
    }

    #[test]
    fn parse_extracts_affected_range() {
        let v = parse_response(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v[0].affected.len(), 1);
        assert_eq!(v[0].affected[0].purl_type, "npm");
        assert_eq!(v[0].affected[0].name, "lodash");
        assert_eq!(v[0].affected[0].vers, "<4.17.12");
        assert_eq!(v[0].affected[0].fixed.as_deref(), Some("4.17.12"));
    }

    #[test]
    fn parse_extracts_cwes() {
        let v = parse_response(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v[0].cwes, vec![1321]);
    }

    #[test]
    fn parse_falls_back_to_snyk_id_when_no_cve() {
        let blob = r#"{"data":[{"id":"SNYK-X","attributes":{
          "title":"t","description":"d","severities":[{"level":"low","score":2.0}],
          "problems":[],"coordinates":[],"created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"
        }}]}"#;
        let v = parse_response(blob.as_bytes()).unwrap();
        assert_eq!(v[0].vuln_id, "SNYK-X");
    }
}
