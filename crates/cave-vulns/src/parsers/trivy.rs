// SPDX-License-Identifier: AGPL-3.0-or-later
//! Trivy parser (container/IaC/secret scanner).
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/tools/trivy/parser.py
//!         (`class TrivyParser`).
//!
//! Wire format: `trivy image -f json` →
//! ```json
//! {"Results": [{
//!   "Target": "alpine:3.18 (alpine 3.18)",
//!   "Type": "alpine",
//!   "Vulnerabilities": [{
//!     "VulnerabilityID": "CVE-…", "PkgName": "libssl", "InstalledVersion": "…",
//!     "FixedVersion": "…", "Severity": "HIGH", "Title": "…",
//!     "Description": "…", "PrimaryURL": "…", "CweIDs": ["CWE-79"],
//!     "CVSS": {"nvd": {"V3Vector": "CVSS:3.1/…", "V3Score": 9.5}}
//!   }],
//!   "Secrets": [{ … }],
//!   "Misconfigurations": [{ … }]
//! }]}
//! ```

use super::{ParserError, ScanParser};
use crate::cvss::severity_from_score;
use crate::finding::{Finding, FindingSeverity};
use serde::Deserialize;
use serde_json::Value;

pub struct TrivyParser;

#[derive(Deserialize)]
struct Report {
    #[serde(rename = "Results", default)]
    results: Vec<TgtResult>,
}
#[derive(Deserialize)]
struct TgtResult {
    #[serde(rename = "Target", default)]
    target: String,
    #[serde(rename = "Type", default)]
    target_type: String,
    #[serde(rename = "Vulnerabilities", default)]
    vulnerabilities: Vec<TVuln>,
    #[serde(rename = "Secrets", default)]
    secrets: Vec<TSecret>,
    #[serde(rename = "Misconfigurations", default)]
    misconfigurations: Vec<TMisc>,
}
#[derive(Deserialize)]
struct TVuln {
    #[serde(rename = "VulnerabilityID", default)]
    vulnerability_id: String,
    #[serde(rename = "PkgName", default)]
    pkg_name: String,
    #[serde(rename = "InstalledVersion", default)]
    installed_version: String,
    #[serde(rename = "FixedVersion", default)]
    fixed_version: Option<String>,
    #[serde(rename = "Severity", default)]
    severity: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(rename = "Description", default)]
    description: Option<String>,
    #[serde(rename = "PrimaryURL", default)]
    primary_url: Option<String>,
    #[serde(rename = "CweIDs", default)]
    cwe_ids: Vec<String>,
    #[serde(rename = "CVSS", default)]
    cvss: Value,
}
#[derive(Deserialize)]
struct TSecret {
    #[serde(rename = "RuleID", default)]
    rule_id: String,
    #[serde(rename = "Severity", default)]
    severity: String,
    #[serde(rename = "Title", default)]
    title: Option<String>,
    #[serde(rename = "Match", default)]
    matched: Option<String>,
    #[serde(rename = "Category", default)]
    category: Option<String>,
}
#[derive(Deserialize)]
struct TMisc {
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "Title", default)]
    title: Option<String>,
    #[serde(rename = "Description", default)]
    description: Option<String>,
    #[serde(rename = "Severity", default)]
    severity: String,
}

fn trivy_severity(s: &str) -> FindingSeverity {
    // Source: TRIVY_SEVERITIES table in upstream parser.py:15-21
    match s.to_ascii_uppercase().as_str() {
        "CRITICAL" => FindingSeverity::Critical,
        "HIGH" => FindingSeverity::High,
        "MEDIUM" => FindingSeverity::Medium,
        "LOW" => FindingSeverity::Low,
        _ => FindingSeverity::Info, // UNKNOWN / blank
    }
}

fn pick_cvss(cvss: &Value) -> (Option<String>, Option<f32>) {
    // Source: CVSS_SEVERITY_SOURCES ["nvd","ghsa","redhat","bitnami"]
    //         in upstream parser.py:23-28.
    for src in ["nvd", "ghsa", "redhat", "bitnami"] {
        if let Some(o) = cvss.get(src) {
            let v3v = o.get("V3Vector").and_then(|v| v.as_str()).map(String::from);
            let v3s = o.get("V3Score").and_then(|v| v.as_f64()).map(|x| x as f32);
            if v3v.is_some() || v3s.is_some() { return (v3v, v3s); }
        }
    }
    (None, None)
}

impl ScanParser for TrivyParser {
    fn scan_type(&self) -> &'static str { "Trivy Scan" }
    fn dedupe_fields(&self) -> &'static [&'static str] {
        &["title", "severity", "vulnerability_ids", "cwe", "description"]
    }
    fn parse(&self, data: &[u8]) -> Result<Vec<Finding>, ParserError> {
        let report: Report = serde_json::from_slice(data)?;
        let mut out = Vec::new();
        for r in report.results {
            // Vulnerabilities → standard SCA findings
            for v in r.vulnerabilities {
                let title = v.title.clone().unwrap_or_else(|| v.vulnerability_id.clone());
                let mut sev = trivy_severity(&v.severity);
                let (vec, score) = pick_cvss(&v.cvss);
                if let Some(s) = score {
                    let promoted = severity_from_score(s);
                    if promoted.weight() > sev.weight() {
                        sev = promoted;
                    }
                }
                let mut f = Finding::new(title.clone(), sev);
                f.cve = if v.vulnerability_id.starts_with("CVE-") { Some(v.vulnerability_id.clone()) } else { None };
                f.vulnerability_ids = vec![v.vulnerability_id.clone()];
                f.vuln_id_from_tool = Some(v.vulnerability_id.clone());
                f.cvssv3 = vec;
                f.cvssv3_score = score;
                f.component_name = Some(v.pkg_name);
                f.component_version = Some(v.installed_version);
                f.fix_version = v.fixed_version.clone();
                f.fix_available = Some(v.fixed_version.is_some());
                f.description = format!(
                    "{title}\n**Target:** {tgt}\n**Type:** {ty}\n**Fixed version:** {fix}\n\n{desc}",
                    title = title,
                    tgt = r.target,
                    ty = r.target_type,
                    fix = f.fix_version.clone().unwrap_or_else(|| "n/a".into()),
                    desc = v.description.unwrap_or_default(),
                );
                if let Some(u) = v.primary_url { f.references = Some(u); }
                if let Some(cwe) = v.cwe_ids.first().and_then(|s| s.to_ascii_uppercase().strip_prefix("CWE-").map(String::from)) {
                    if let Ok(n) = cwe.parse() { f.cwe = Some(n); }
                }
                f.found_by_scanner = Some("Trivy Scan".into());
                f.service = Some(r.target.clone());
                out.push(f);
            }
            for s in r.secrets {
                let title = s.title.clone().unwrap_or_else(|| s.rule_id.clone());
                let sev = trivy_severity(&s.severity);
                let mut f = Finding::new(title.clone(), sev);
                f.vuln_id_from_tool = Some(s.rule_id.clone());
                f.description = format!(
                    "{title}\n**Category:** {cat}\n**Match:** {m}",
                    title = title,
                    cat = s.category.unwrap_or_default(),
                    m = s.matched.unwrap_or_default(),
                );
                f.found_by_scanner = Some("Trivy Scan".into());
                f.service = Some(r.target.clone());
                out.push(f);
            }
            for m in r.misconfigurations {
                let title = m.title.clone().unwrap_or_else(|| m.id.clone());
                let sev = trivy_severity(&m.severity);
                let mut f = Finding::new(title.clone(), sev);
                f.vuln_id_from_tool = Some(m.id.clone());
                f.description = m.description.unwrap_or_default();
                f.found_by_scanner = Some("Trivy Scan".into());
                f.service = Some(r.target.clone());
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
      "Results": [{
        "Target": "alpine:3.18",
        "Type": "alpine",
        "Vulnerabilities": [{
          "VulnerabilityID": "CVE-2024-99999",
          "PkgName": "openssl", "InstalledVersion": "1.1.1",
          "FixedVersion": "1.1.1w-r3",
          "Severity": "HIGH", "Title": "OpenSSL flaw",
          "Description": "Memory leak", "PrimaryURL": "https://nvd.nist.gov/CVE-2024-99999",
          "CweIDs": ["CWE-401"],
          "CVSS": {"nvd": {"V3Vector": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H", "V3Score": 9.8}}
        }],
        "Secrets": [{
          "RuleID": "aws-access-key", "Severity": "CRITICAL",
          "Title": "AWS Access Key", "Match": "AKIA****", "Category": "AWS"
        }],
        "Misconfigurations": [{
          "ID": "AVD-DS-0001", "Title": "Last user is root",
          "Description": "Image runs as root", "Severity": "MEDIUM"
        }]
      }]
    }"#;

    #[test]
    fn parses_vuln_secret_misc() {
        let out = TrivyParser.parse(SAMPLE).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn cvss_score_promotes_severity_to_critical() {
        // HIGH text + 9.8 score → Critical
        let out = TrivyParser.parse(SAMPLE).unwrap();
        let vuln = out.iter().find(|f| f.cve.is_some()).unwrap();
        assert_eq!(vuln.severity, FindingSeverity::Critical);
        assert_eq!(vuln.cvssv3_score, Some(9.8));
    }

    #[test]
    fn cve_id_extracted_to_cve_and_vuln_id_from_tool() {
        let out = TrivyParser.parse(SAMPLE).unwrap();
        let vuln = out.iter().find(|f| f.cve.is_some()).unwrap();
        assert_eq!(vuln.cve.as_deref(), Some("CVE-2024-99999"));
        assert_eq!(vuln.vuln_id_from_tool.as_deref(), Some("CVE-2024-99999"));
    }

    #[test]
    fn cwe_extracted_from_first_id() {
        let out = TrivyParser.parse(SAMPLE).unwrap();
        let vuln = out.iter().find(|f| f.cve.is_some()).unwrap();
        assert_eq!(vuln.cwe, Some(401));
    }

    #[test]
    fn fix_version_and_available_set() {
        let out = TrivyParser.parse(SAMPLE).unwrap();
        let vuln = out.iter().find(|f| f.cve.is_some()).unwrap();
        assert_eq!(vuln.fix_version.as_deref(), Some("1.1.1w-r3"));
        assert_eq!(vuln.fix_available, Some(true));
    }

    #[test]
    fn component_name_and_version_extracted() {
        let out = TrivyParser.parse(SAMPLE).unwrap();
        let vuln = out.iter().find(|f| f.cve.is_some()).unwrap();
        assert_eq!(vuln.component_name.as_deref(), Some("openssl"));
        assert_eq!(vuln.component_version.as_deref(), Some("1.1.1"));
    }

    #[test]
    fn secret_finding_parsed() {
        let out = TrivyParser.parse(SAMPLE).unwrap();
        let secret = out.iter().find(|f| f.title == "AWS Access Key").unwrap();
        assert_eq!(secret.severity, FindingSeverity::Critical);
    }

    #[test]
    fn misconfig_parsed() {
        let out = TrivyParser.parse(SAMPLE).unwrap();
        let misc = out.iter().find(|f| f.title == "Last user is root").unwrap();
        assert_eq!(misc.severity, FindingSeverity::Medium);
    }

    #[test]
    fn unknown_severity_falls_to_info() {
        assert_eq!(trivy_severity("UNKNOWN"), FindingSeverity::Info);
        assert_eq!(trivy_severity(""), FindingSeverity::Info);
    }
}
