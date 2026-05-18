// SPDX-License-Identifier: AGPL-3.0-or-later
//! Snyk CLI JSON parser (`snyk test --json`).
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/tools/snyk/parser.py
//!         (`class SnykParser`). DefectDojo additionally has a
//!         dedicated SnykCodeParser for `snyk code` SARIF; we route
//!         that to the SARIF parser per upstream behaviour.
//!
//! Wire format: top-level object with `vulnerabilities: [{ id,
//! title, packageName, version, severity, CVSSv3?, cvssScore?,
//! identifiers: { CVE?, CWE? }, description?, epssDetails? }]`.

use super::{ParserError, ScanParser};
use crate::cvss::severity_from_score;
use crate::finding::{Finding, FindingSeverity};
use serde::Deserialize;
use serde_json::Value;

pub struct SnykParser;

#[derive(Deserialize)]
struct Report {
    #[serde(default)]
    vulnerabilities: Vec<SnVuln>,
}
#[derive(Deserialize)]
struct SnVuln {
    id: String,
    title: String,
    #[serde(default, rename = "packageName")]
    package_name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    severity: String,
    #[serde(default, rename = "CVSSv3")]
    cvssv3: Option<String>,
    #[serde(default, rename = "cvssScore")]
    cvss_score: Option<f32>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    identifiers: Value,
    #[serde(default, rename = "epssDetails")]
    epss_details: Value,
}

impl ScanParser for SnykParser {
    fn scan_type(&self) -> &'static str { "Snyk Scan" }
    fn dedupe_fields(&self) -> &'static [&'static str] {
        &["vuln_id_from_tool", "file_path", "component_name", "component_version"]
    }
    fn parse(&self, data: &[u8]) -> Result<Vec<Finding>, ParserError> {
        // Snyk reports may be a single object OR a list (multi-module).
        let trimmed = data.iter().position(|b| !b.is_ascii_whitespace()).map(|i| data[i]).unwrap_or(b'{');
        let reports: Vec<Report> = if trimmed == b'[' {
            serde_json::from_slice::<Vec<Report>>(data)?
        } else {
            vec![serde_json::from_slice::<Report>(data)?]
        };
        let mut out = Vec::new();
        for rep in reports {
            for v in rep.vulnerabilities {
                let mut sev = FindingSeverity::parse(&v.severity).unwrap_or(FindingSeverity::Info);
                if let Some(s) = v.cvss_score {
                    let promoted = severity_from_score(s);
                    if promoted.weight() > sev.weight() { sev = promoted; }
                }
                let mut f = Finding::new(v.title, sev);
                f.vuln_id_from_tool = Some(v.id.clone());
                f.cvssv3 = v.cvssv3;
                f.cvssv3_score = v.cvss_score;
                f.component_name = v.package_name;
                f.component_version = v.version;
                f.description = v.description.unwrap_or_default();
                if let Some(cves) = v.identifiers.get("CVE").and_then(|c| c.as_array()) {
                    let cve_list: Vec<String> = cves.iter().filter_map(|c| c.as_str().map(String::from)).collect();
                    if let Some(c) = cve_list.first() {
                        f.cve = Some(c.clone());
                    }
                    f.vulnerability_ids = cve_list;
                }
                if let Some(cwes) = v.identifiers.get("CWE").and_then(|c| c.as_array()) {
                    if let Some(first) = cwes.iter().filter_map(|c| c.as_str()).next() {
                        if let Some(n) = first.to_ascii_uppercase().strip_prefix("CWE-").and_then(|s| s.parse().ok()) {
                            f.cwe = Some(n);
                        }
                    }
                }
                if let Some(score) = v.epss_details.get("score").and_then(|v| v.as_f64()) {
                    f.epss_score = Some(score as f32);
                }
                if let Some(pct) = v.epss_details.get("percentile").and_then(|v| v.as_f64()) {
                    f.epss_percentile = Some(pct as f32);
                }
                f.found_by_scanner = Some("Snyk Scan".into());
                f.static_finding = true;
                f.dynamic_finding = false;
                out.push(f);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"{
        "vulnerabilities": [
            {"id": "SNYK-JS-LODASH-1018905", "title": "Prototype Pollution",
             "packageName": "lodash", "version": "4.17.15",
             "severity": "high", "CVSSv3": "CVSS:3.1/AV:N/AC:H/PR:N/UI:N/S:U/C:H/I:H/A:H",
             "cvssScore": 7.5,
             "description": "Prototype pollution in merge",
             "identifiers": {"CVE": ["CVE-2020-8203"], "CWE": ["CWE-1321"]},
             "epssDetails": {"score": 0.12345, "percentile": 0.99}}
        ]
    }"#;

    #[test]
    fn parses_single_vulnerability() {
        let out = SnykParser.parse(SAMPLE).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn severity_from_score_promotes() {
        let out = SnykParser.parse(SAMPLE).unwrap();
        // text "high" + score 7.5 → High (no promotion past)
        assert_eq!(out[0].severity, FindingSeverity::High);
    }

    #[test]
    fn cve_from_identifiers_first_entry() {
        let out = SnykParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].cve.as_deref(), Some("CVE-2020-8203"));
        assert_eq!(out[0].vulnerability_ids, vec!["CVE-2020-8203".to_string()]);
    }

    #[test]
    fn cwe_extracted_from_identifiers() {
        let out = SnykParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].cwe, Some(1321));
    }

    #[test]
    fn component_name_and_version_set() {
        let out = SnykParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].component_name.as_deref(), Some("lodash"));
        assert_eq!(out[0].component_version.as_deref(), Some("4.17.15"));
    }

    #[test]
    fn cvss_vector_preserved() {
        let out = SnykParser.parse(SAMPLE).unwrap();
        assert!(out[0].cvssv3.as_deref().unwrap().starts_with("CVSS:3.1"));
    }

    #[test]
    fn epss_score_and_percentile() {
        let out = SnykParser.parse(SAMPLE).unwrap();
        assert!((out[0].epss_score.unwrap() - 0.12345).abs() < 1e-4);
        assert!((out[0].epss_percentile.unwrap() - 0.99).abs() < 1e-4);
    }

    #[test]
    fn handles_multi_module_array_top_level() {
        let s = br#"[{"vulnerabilities":[{"id":"A","title":"x","severity":"low","identifiers":{}}]},
                     {"vulnerabilities":[{"id":"B","title":"y","severity":"low","identifiers":{}}]}]"#;
        let out = SnykParser.parse(s).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn empty_vulnerabilities_returns_empty() {
        let out = SnykParser.parse(br#"{"vulnerabilities":[]}"#).unwrap();
        assert!(out.is_empty());
    }
}
