// SPDX-License-Identifier: AGPL-3.0-or-later
//! Semgrep JSON parser.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/tools/semgrep/parser.py
//!         (`class SemgrepParser`).
//!
//! Two shapes:
//!   1. `semgrep --json` → `{"results":[{check_id, path, start.line, extra:{severity,message,metadata:{cwe?,references?},fix?,fingerprint?}}]}`
//!   2. `semgrep ci` SCA → `{"vulns":[{title, advisory:{severity,description,references:{cweIds?}}, dependencyFileLocation:{path,startLine}, repositoryId}]}`

use super::{ParserError, ScanParser};
use crate::finding::{Finding, FindingSeverity};
use serde::Deserialize;
use serde_json::Value;

pub struct SemgrepParser;

#[derive(Deserialize)]
struct Report {
    #[serde(default)]
    results: Vec<SgResult>,
    #[serde(default)]
    vulns: Vec<SgVuln>,
}

#[derive(Deserialize)]
struct SgResult {
    check_id: String,
    path: String,
    start: SgStart,
    extra: SgExtra,
}
#[derive(Deserialize)]
struct SgStart { line: u32 }
#[derive(Deserialize)]
struct SgExtra {
    severity: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    fingerprint: Option<String>,
    #[serde(default)]
    fix: Option<String>,
}

#[derive(Deserialize)]
struct SgVuln {
    title: String,
    advisory: SgAdvisory,
    #[serde(rename = "dependencyFileLocation")]
    location: SgLocation,
    #[serde(rename = "repositoryId")]
    repo_id: String,
}
#[derive(Deserialize)]
struct SgAdvisory {
    severity: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    references: Value,
}
#[derive(Deserialize)]
struct SgLocation { path: String, #[serde(rename = "startLine")] start_line: u32 }

impl SemgrepParser {
    fn convert_severity(s: &str) -> FindingSeverity {
        match s.to_ascii_uppercase().as_str() {
            "CRITICAL" => FindingSeverity::Critical,
            "WARNING" | "MEDIUM" => FindingSeverity::Medium,
            "ERROR" | "HIGH" => FindingSeverity::High,
            "LOW" | "INFO" => FindingSeverity::Low,
            _ => FindingSeverity::Info,
        }
    }
}

impl ScanParser for SemgrepParser {
    fn scan_type(&self) -> &'static str { "Semgrep JSON Report" }
    fn dedupe_fields(&self) -> &'static [&'static str] {
        &["title", "cwe", "line", "file_path", "description"]
    }
    fn parse(&self, data: &[u8]) -> Result<Vec<Finding>, ParserError> {
        let report: Report = serde_json::from_slice(data)?;
        let mut out = Vec::new();
        for r in report.results {
            let sev = Self::convert_severity(&r.extra.severity);
            let mut f = Finding::new(r.check_id.clone(), sev);
            f.file_path = Some(r.path);
            f.line = Some(r.start.line);
            f.vuln_id_from_tool = Some(r.check_id);
            f.static_finding = true;
            f.found_by_scanner = Some("Semgrep JSON Report".into());
            if let Some(msg) = r.extra.message {
                f.description = format!("**Result message:** {msg}");
            }
            // CWE may be `["CWE-79: Cross-site Scripting"]` or `"CWE-79: …"`.
            if let Some(cwe_val) = r.extra.metadata.get("cwe") {
                let cwe_str = match cwe_val {
                    Value::Array(a) => a.first().and_then(|v| v.as_str()).map(String::from),
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                };
                if let Some(s) = cwe_str {
                    // "CWE-79: …" → 79
                    let trimmed = s.split(':').next().unwrap_or("").to_ascii_uppercase();
                    if let Some(n) = trimmed.strip_prefix("CWE-").and_then(|x| x.trim().parse().ok()) {
                        f.cwe = Some(n);
                    }
                }
            }
            if let Some(refs) = r.extra.metadata.get("references").and_then(|v| v.as_array()) {
                let lines: Vec<String> = refs.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                if !lines.is_empty() { f.references = Some(lines.join("\n")); }
            }
            if let Some(fix) = r.extra.fix {
                f.mitigation = Some(fix);
            }
            if let Some(fp) = r.extra.fingerprint {
                if fp != "requires login" {
                    f.unique_id_from_tool = Some(fp);
                }
            }
            out.push(f);
        }
        for v in report.vulns {
            let sev = Self::convert_severity(&v.advisory.severity);
            let mut f = Finding::new(v.title, sev);
            f.file_path = Some(v.location.path);
            f.line = Some(v.location.start_line);
            f.vuln_id_from_tool = Some(v.repo_id);
            f.static_finding = true;
            f.found_by_scanner = Some("Semgrep JSON Report".into());
            if let Some(d) = v.advisory.description { f.description = d; }
            if let Some(cwe_ids) = v.advisory.references.get("cweIds") {
                let cwe_str = match cwe_ids {
                    Value::Array(a) => a.first().and_then(|v| v.as_str()).map(String::from),
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                };
                if let Some(s) = cwe_str {
                    let trimmed = s.split(':').next().unwrap_or("").to_ascii_uppercase();
                    if let Some(n) = trimmed.strip_prefix("CWE-").and_then(|x| x.trim().parse().ok()) {
                        f.cwe = Some(n);
                    }
                }
            }
            out.push(f);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESULTS_SAMPLE: &[u8] = br#"{
        "results": [
            {"check_id": "python.lang.security.audit.hardcoded-password",
             "path": "src/x.py", "start": {"line": 12},
             "extra": {"severity": "ERROR", "message": "hardcoded",
                       "metadata": {"cwe": ["CWE-798: Use of Hard-coded Credentials"],
                                    "references": ["https://cwe.mitre.org/data/definitions/798.html"]},
                       "fix": "Use env var",
                       "fingerprint": "abc123"}},
            {"check_id": "xss.tainted",
             "path": "src/y.py", "start": {"line": 5},
             "extra": {"severity": "WARNING", "message": "xss",
                       "metadata": {"cwe": "CWE-79"}}}
        ]
    }"#;

    #[test]
    fn parses_results_shape() {
        let out = SemgrepParser.parse(RESULTS_SAMPLE).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn maps_error_to_high_warning_to_medium() {
        let out = SemgrepParser.parse(RESULTS_SAMPLE).unwrap();
        assert_eq!(out[0].severity, FindingSeverity::High);
        assert_eq!(out[1].severity, FindingSeverity::Medium);
    }

    #[test]
    fn extracts_cwe_from_array_and_string() {
        let out = SemgrepParser.parse(RESULTS_SAMPLE).unwrap();
        assert_eq!(out[0].cwe, Some(798));
        assert_eq!(out[1].cwe, Some(79));
    }

    #[test]
    fn extracts_fingerprint_as_unique_id() {
        let out = SemgrepParser.parse(RESULTS_SAMPLE).unwrap();
        assert_eq!(out[0].unique_id_from_tool.as_deref(), Some("abc123"));
        assert!(out[1].unique_id_from_tool.is_none());
    }

    #[test]
    fn extracts_fix_as_mitigation() {
        let out = SemgrepParser.parse(RESULTS_SAMPLE).unwrap();
        assert_eq!(out[0].mitigation.as_deref(), Some("Use env var"));
    }

    #[test]
    fn parses_vulns_shape() {
        let s = br#"{"vulns": [
            {"title": "Dep CVE", "advisory": {"severity": "CRITICAL", "description": "boom",
                "references": {"cweIds": "CWE-89"}},
             "dependencyFileLocation": {"path": "package.json", "startLine": 1},
             "repositoryId": "npm:lodash"}
        ]}"#;
        let out = SemgrepParser.parse(s).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, FindingSeverity::Critical);
        assert_eq!(out[0].cwe, Some(89));
        assert_eq!(out[0].vuln_id_from_tool.as_deref(), Some("npm:lodash"));
    }

    #[test]
    fn ignores_fingerprint_requires_login() {
        let s = br#"{"results": [{"check_id":"x","path":"a","start":{"line":1},
            "extra":{"severity":"INFO","metadata":{},"fingerprint":"requires login"}}]}"#;
        let out = SemgrepParser.parse(s).unwrap();
        assert!(out[0].unique_id_from_tool.is_none());
    }
}
