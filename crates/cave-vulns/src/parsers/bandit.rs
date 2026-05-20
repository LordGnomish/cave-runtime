// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bandit (Python SAST) parser.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/tools/bandit/parser.py
//!         (`class BanditParser`).
//!
//! Wire format: `bandit -f json -o out.json …` → top-level object
//! with `results: [{ test_name, test_id, filename, line_number,
//! issue_severity, issue_confidence, issue_text, code, more_info? }]`.

use super::{ParserError, ScanParser};
use crate::finding::{Finding, FindingSeverity};
use chrono::Utc;
use serde::Deserialize;

pub struct BanditParser;

#[derive(Deserialize)]
struct BanditReport {
    #[serde(default)]
    results: Vec<BanditIssue>,
}

#[derive(Deserialize)]
struct BanditIssue {
    test_name: String,
    test_id: String,
    filename: String,
    line_number: u32,
    issue_severity: String,
    issue_confidence: Option<String>,
    issue_text: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    more_info: Option<String>,
}

impl ScanParser for BanditParser {
    fn scan_type(&self) -> &'static str {
        "Bandit Scan"
    }

    // Source: dojo/tools/bandit/parser.py::get_dedupe_fields
    fn dedupe_fields(&self) -> &'static [&'static str] {
        &["file_path", "line", "vuln_id_from_tool"]
    }

    fn parse(&self, data: &[u8]) -> Result<Vec<Finding>, ParserError> {
        let report: BanditReport = serde_json::from_slice(data)?;
        let now = Utc::now();
        let mut out = Vec::with_capacity(report.results.len());
        for item in report.results {
            // Source: dojo/tools/bandit/parser.py:91
            //   severity = item["issue_severity"].title()
            let sev = FindingSeverity::parse(&item.issue_severity).unwrap_or(FindingSeverity::Info);
            let vuln_id = format!("{}:{}", item.test_name, item.test_id);
            let mut f = Finding::new(item.issue_text.clone(), sev);
            f.date = now;
            f.file_path = Some(item.filename);
            f.line = Some(item.line_number);
            f.vuln_id_from_tool = Some(vuln_id);
            f.static_finding = true;
            f.dynamic_finding = false;
            f.found_by_scanner = Some("Bandit Scan".into());
            // Custom description from upstream parser lines 73-86.
            f.description = format!(
                "**Test Name:** `{tn}`\n**Test ID:** `{tid}`\n**Filename:** `{fp}`\n**Line:** `{ln}`\n**Confidence:** `{c}`\n\n```\n{code}\n```",
                tn = item.test_name,
                tid = item.test_id,
                fp = f.file_path.clone().unwrap_or_default(),
                ln = item.line_number,
                c = item.issue_confidence.clone().unwrap_or_default(),
                code = item.code.unwrap_or_default(),
            );
            if let Some(info) = item.more_info {
                f.references = Some(info);
            }
            out.push(f);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"{
        "results": [
            {"test_name": "hardcoded_password_string", "test_id": "B105",
             "filename": "src/auth.py", "line_number": 42,
             "issue_severity": "HIGH", "issue_confidence": "MEDIUM",
             "issue_text": "Possible hardcoded password", "code": "password = 'admin'",
             "more_info": "https://bandit.readthedocs.io/B105"},
            {"test_name": "assert_used", "test_id": "B101",
             "filename": "src/x.py", "line_number": 10,
             "issue_severity": "LOW", "issue_confidence": "HIGH",
             "issue_text": "assert detected"}
        ]
    }"#;

    #[test]
    fn parses_two_findings() {
        let out = BanditParser.parse(SAMPLE).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn maps_severity_titlecase() {
        let out = BanditParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].severity, FindingSeverity::High);
        assert_eq!(out[1].severity, FindingSeverity::Low);
    }

    #[test]
    fn populates_file_and_line() {
        let out = BanditParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].file_path.as_deref(), Some("src/auth.py"));
        assert_eq!(out[0].line, Some(42));
    }

    #[test]
    fn builds_vuln_id_from_tool_as_name_colon_id() {
        let out = BanditParser.parse(SAMPLE).unwrap();
        assert_eq!(
            out[0].vuln_id_from_tool.as_deref(),
            Some("hardcoded_password_string:B105")
        );
        assert_eq!(
            out[1].vuln_id_from_tool.as_deref(),
            Some("assert_used:B101")
        );
    }

    #[test]
    fn marks_as_static_finding() {
        let out = BanditParser.parse(SAMPLE).unwrap();
        assert!(out[0].static_finding);
        assert!(!out[0].dynamic_finding);
    }

    #[test]
    fn empty_results_returns_empty_vec() {
        let out = BanditParser.parse(br#"{"results": []}"#).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn rejects_garbage() {
        assert!(BanditParser.parse(b"not json").is_err());
    }

    #[test]
    fn scan_type_matches_defectdojo() {
        assert_eq!(BanditParser.scan_type(), "Bandit Scan");
    }
}
