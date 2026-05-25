// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/osv/OsvAdvisoryParser.java
//   ossf.github.io/osv-schema (spec reference)
//
//! OSV.dev v1.6 advisory parser.

use crate::models::{AffectedRange, AnalysisState, Severity, VulnIntel, VulnSource};
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum OsvError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct OsvAdvisory {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    severity: Vec<OsvSeverity>,
    #[serde(default)]
    affected: Vec<OsvAffected>,
    #[serde(default)]
    references: Vec<OsvRef>,
    published: Option<String>,
    modified: Option<String>,
    #[serde(default)]
    database_specific: Option<OsvDatabaseSpecific>,
}

#[derive(Debug, Deserialize)]
struct OsvSeverity {
    #[serde(rename = "type")]
    sev_type: String,
    score: String,
}

#[derive(Debug, Deserialize)]
struct OsvAffected {
    package: Option<OsvPackage>,
    #[serde(default)]
    ranges: Vec<OsvRange>,
}

#[derive(Debug, Deserialize)]
struct OsvPackage {
    ecosystem: String,
    name: String,
    purl: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OsvRange {
    #[serde(rename = "type")]
    range_type: String,
    #[serde(default)]
    events: Vec<OsvEvent>,
}

#[derive(Debug, Deserialize)]
struct OsvEvent {
    introduced: Option<String>,
    fixed: Option<String>,
    #[serde(default)]
    last_affected: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OsvRef {
    #[serde(rename = "type")]
    _ref_type: Option<String>,
    url: String,
}

#[derive(Debug, Deserialize)]
struct OsvDatabaseSpecific {
    #[serde(default)]
    cwe_ids: Vec<String>,
}

/// Parse a single OSV advisory.
pub fn parse_advisory(input: &[u8]) -> Result<VulnIntel, OsvError> {
    let a: OsvAdvisory = serde_json::from_slice(input)?;
    Ok(to_intel(a))
}

/// Parse a JSONL stream of advisories (one per line).
pub fn parse_jsonl(input: &[u8]) -> Result<Vec<VulnIntel>, OsvError> {
    let mut out = Vec::new();
    for line in input.split(|b| *b == b'\n') {
        let trimmed = line
            .iter()
            .copied()
            .skip_while(|b| b.is_ascii_whitespace())
            .collect::<Vec<_>>();
        if trimmed.is_empty() {
            continue;
        }
        let a: OsvAdvisory = serde_json::from_slice(&trimmed)?;
        out.push(to_intel(a));
    }
    Ok(out)
}

fn to_intel(a: OsvAdvisory) -> VulnIntel {
    // Prefer a CVE alias as primary id if present.
    let primary = a
        .aliases
        .iter()
        .find(|x| x.starts_with("CVE-"))
        .cloned()
        .unwrap_or_else(|| a.id.clone());
    let cvss_vector = a
        .severity
        .iter()
        .find(|s| s.sev_type == "CVSS_V3")
        .map(|s| s.score.clone());
    let cvss_v3_base = cvss_vector.as_deref().and_then(parse_cvss_v3_base);
    let severity = cvss_v3_base
        .map(Severity::from_cvss_v3)
        .unwrap_or(Severity::Unassigned);
    let cwes = a
        .database_specific
        .as_ref()
        .map(|d| {
            d.cwe_ids
                .iter()
                .filter_map(|s| s.strip_prefix("CWE-").and_then(|n| n.parse::<u32>().ok()))
                .collect()
        })
        .unwrap_or_default();
    let references = a.references.iter().map(|r| r.url.clone()).collect();
    let mut affected = Vec::new();
    for af in &a.affected {
        let (purl_type, namespace, name) = match (&af.package, ()) {
            (Some(p), _) => (purl_type_from_ecosystem(&p.ecosystem), None, p.name.clone()),
            (None, _) => ("unknown".to_string(), None, String::new()),
        };
        for r in &af.ranges {
            let (vers, fixed) = events_to_vers(&r.events);
            affected.push(AffectedRange {
                purl_type: purl_type.clone(),
                namespace: namespace.clone(),
                name: name.clone(),
                vers,
                fixed,
            });
        }
    }
    VulnIntel {
        id: Uuid::new_v4(),
        vuln_id: primary,
        source: VulnSource::Osv,
        title: a.summary.unwrap_or_default(),
        description: a.details.unwrap_or_default(),
        severity,
        cvss_v3_base,
        cvss_v3_vector: cvss_vector,
        cvss_v2_base: None,
        epss_score: None,
        epss_percentile: None,
        cwes,
        references,
        affected,
        published: parse_dt(a.published.as_deref()),
        modified: parse_dt(a.modified.as_deref()),
        state: AnalysisState::NotSet,
    }
}

fn events_to_vers(events: &[OsvEvent]) -> (String, Option<String>) {
    // OSV events come in order: `introduced` then `fixed`/`last_affected`.
    let mut intro: Option<String> = None;
    let mut fixed: Option<String> = None;
    let mut last_affected: Option<String> = None;
    for e in events {
        if let Some(i) = &e.introduced {
            intro = Some(i.clone());
        }
        if let Some(f) = &e.fixed {
            fixed = Some(f.clone());
        }
        if let Some(la) = &e.last_affected {
            last_affected = Some(la.clone());
        }
    }
    let lower = intro.as_ref().map(|i| {
        if i == "0" {
            String::new()
        } else {
            format!(">={}", i)
        }
    });
    let upper = if let Some(f) = &fixed {
        Some(format!("<{}", f))
    } else {
        last_affected.as_ref().map(|la| format!("<={}", la))
    };
    let vers = match (
        lower.as_deref().unwrap_or(""),
        upper.as_deref().unwrap_or(""),
    ) {
        ("", "") => "*".to_string(),
        ("", up) => up.to_string(),
        (lo, "") => lo.to_string(),
        (lo, up) => format!("{} {}", lo, up),
    };
    (vers, fixed)
}

fn purl_type_from_ecosystem(eco: &str) -> String {
    // Mapping from OSV ecosystem names to PURL types.
    match eco.to_ascii_lowercase().as_str() {
        "npm" => "npm".into(),
        "pypi" => "pypi".into(),
        "go" => "golang".into(),
        "maven" => "maven".into(),
        "rubygems" => "gem".into(),
        "nuget" => "nuget".into(),
        "packagist" => "composer".into(),
        "crates.io" => "cargo".into(),
        "hex" => "hex".into(),
        other => other.to_string(),
    }
}

fn parse_cvss_v3_base(vector: &str) -> Option<f32> {
    // OSV's CVSS_V3 score field is the full vector string ("CVSS:3.1/AV:N/...").
    // No base score field; for honest parity we cannot compute here without
    // a CVSS implementation. Return None — callers may compute upstream.
    let _ = vector;
    None
}

fn parse_dt(s: Option<&str>) -> Option<chrono::DateTime<chrono::Utc>> {
    s.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "id": "GHSA-jf85-cpcp-j695",
      "aliases": ["CVE-2019-10744"],
      "summary": "Prototype Pollution in lodash",
      "details": "Versions of lodash prior to 4.17.12 are vulnerable to ...",
      "severity": [{ "type":"CVSS_V3", "score":"CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H" }],
      "affected": [{
        "package": { "ecosystem":"npm", "name":"lodash" },
        "ranges": [{ "type":"SEMVER",
                     "events":[ {"introduced":"0"}, {"fixed":"4.17.12"} ] }]
      }],
      "references":[ {"type":"WEB","url":"https://example.com"} ],
      "published":"2019-07-19T22:23:00Z",
      "modified":"2024-01-01T00:00:00Z",
      "database_specific": { "cwe_ids":["CWE-1321"] }
    }"#;

    #[test]
    fn parse_uses_cve_alias_as_primary_id() {
        let v = parse_advisory(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v.vuln_id, "CVE-2019-10744");
    }

    #[test]
    fn parse_extracts_cvss_vector() {
        let v = parse_advisory(SAMPLE.as_bytes()).unwrap();
        assert!(v.cvss_v3_vector.as_ref().unwrap().starts_with("CVSS:3.1/"));
    }

    #[test]
    fn parse_extracts_affected_range() {
        let v = parse_advisory(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v.affected.len(), 1);
        assert_eq!(v.affected[0].purl_type, "npm");
        assert_eq!(v.affected[0].name, "lodash");
        assert_eq!(v.affected[0].vers, "<4.17.12");
        assert_eq!(v.affected[0].fixed.as_deref(), Some("4.17.12"));
    }

    #[test]
    fn parse_extracts_cwes_from_database_specific() {
        let v = parse_advisory(SAMPLE.as_bytes()).unwrap();
        assert_eq!(v.cwes, vec![1321]);
    }

    #[test]
    fn parse_jsonl_multiple() {
        // JSONL = one compact JSON object per line.
        let one = r#"{"id":"GHSA-a","aliases":["CVE-2024-1"],"affected":[],"references":[]}"#;
        let two = r#"{"id":"GHSA-b","aliases":["CVE-2024-2"],"affected":[],"references":[]}"#;
        let joined = format!("{}\n{}\n", one, two);
        let v = parse_jsonl(joined.as_bytes()).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].vuln_id, "CVE-2024-1");
        assert_eq!(v[1].vuln_id, "CVE-2024-2");
    }

    #[test]
    fn purl_type_mapping() {
        assert_eq!(purl_type_from_ecosystem("npm"), "npm");
        assert_eq!(purl_type_from_ecosystem("Go"), "golang");
        assert_eq!(purl_type_from_ecosystem("crates.io"), "cargo");
    }
}
