// SPDX-License-Identifier: AGPL-3.0-or-later
//! Generic / universal findings importer.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738
//!         dojo/tools/generic/parser.py (+ json_parser.py, csv_parser.py).
//!
//! DefectDojo's "Generic Findings Import" is the catch-all format every
//! integration without a dedicated parser targets. It accepts two wire
//! formats which we auto-detect from the first non-whitespace byte:
//!
//!   * JSON — `{ "findings": [ { title, severity, description, … } ] }`
//!   * CSV  — a header row drawn from a fixed column vocabulary, one
//!            finding per row.
//!
//! Faithful upstream quirks ported here:
//!   * severity is matched **case-sensitively** against the exact set
//!     `{Info, Low, Medium, High, Critical}`; anything else → `Info`
//!     (so `"CRITICAL"` degrades to `Info`).
//!   * CSV booleans are `value.lower()[0:1] == "t"` — only a leading
//!     `t`/`T` is truthy (`"yes"`, `"1"`, `"false"` are all false).
//!   * CSV rows collapse on `sha256("{severity}|{title}|{description}")`,
//!     incrementing `nb_occurences` on each merge.

use super::{ParserError, ScanParser};
use crate::finding::{Finding, FindingSeverity};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub struct GenericParser;

/// Strict severity normalization — upstream compares the raw string
/// against the canonical TitleCase set and falls back to `Info`.
fn normalize_severity(raw: &str) -> FindingSeverity {
    match raw {
        "Critical" => FindingSeverity::Critical,
        "High" => FindingSeverity::High,
        "Medium" => FindingSeverity::Medium,
        "Low" => FindingSeverity::Low,
        _ => FindingSeverity::Info,
    }
}

/// CSV truthiness — `value.lower()[0:1] == "t"`.
fn csv_bool(raw: &str) -> bool {
    raw.trim()
        .chars()
        .next()
        .map(|c| c.eq_ignore_ascii_case(&'t'))
        .unwrap_or(false)
}

/// Upstream dedup key: `sha256("{severity}|{title}|{description}")`.
pub(crate) fn dedupe_key(sev: FindingSeverity, title: &str, desc: &str) -> String {
    let mut h = Sha256::new();
    h.update(format!("{}|{}|{}", sev.as_str(), title, desc).as_bytes());
    format!("{:x}", h.finalize())
}

#[derive(Deserialize)]
struct GenericReport {
    #[serde(default)]
    findings: Vec<GenericItem>,
}

#[derive(Deserialize)]
struct GenericItem {
    title: String,
    severity: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    cve: Option<String>,
    #[serde(default)]
    cwe: Option<u32>,
    #[serde(default)]
    cvssv3: Option<String>,
    #[serde(default)]
    cvssv3_score: Option<f32>,
    #[serde(default)]
    mitigation: Option<String>,
    #[serde(default)]
    impact: Option<String>,
    #[serde(default)]
    references: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    component_name: Option<String>,
    #[serde(default)]
    component_version: Option<String>,
    #[serde(default)]
    vulnerability_ids: Vec<String>,
    #[serde(default = "default_true")]
    active: bool,
    #[serde(default)]
    verified: bool,
    #[serde(default)]
    false_p: bool,
    #[serde(default)]
    duplicate: bool,
    #[serde(default)]
    is_mitigated: bool,
}

fn default_true() -> bool {
    true
}

impl ScanParser for GenericParser {
    fn scan_type(&self) -> &'static str {
        "Generic Findings Import"
    }
    fn dedupe_fields(&self) -> &'static [&'static str] {
        &["title", "severity", "description"]
    }
    fn parse(&self, data: &[u8]) -> Result<Vec<Finding>, ParserError> {
        // Auto-detect: JSON objects start with '{', everything else is CSV.
        let first = data
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .map(|i| data[i]);
        if first == Some(b'{') {
            parse_json(data)
        } else {
            parse_csv(data)
        }
    }
}

fn finding_from_item(item: GenericItem) -> Finding {
    let sev = normalize_severity(&item.severity);
    let mut f = Finding::new(item.title, sev);
    f.description = item.description;
    f.cwe = item.cwe;
    f.cvssv3 = item.cvssv3;
    f.cvssv3_score = item.cvssv3_score;
    f.mitigation = item.mitigation;
    f.impact = item.impact;
    f.references = item.references;
    f.file_path = item.file_path;
    f.line = item.line;
    f.component_name = item.component_name;
    f.component_version = item.component_version;

    // CVE seeds the vulnerability_ids list; an explicit list wins/merges.
    let mut vids = item.vulnerability_ids;
    if let Some(cve) = item.cve {
        if !vids.contains(&cve) {
            vids.insert(0, cve);
        }
    }
    if let Some(first) = vids.first() {
        f.cve = Some(first.clone());
    }
    f.vulnerability_ids = vids;

    f.state.active = item.active;
    f.state.verified = item.verified;
    f.state.false_p = item.false_p;
    f.state.duplicate = item.duplicate;
    f.state.is_mitigated = item.is_mitigated;
    f.found_by_scanner = Some("Generic Findings Import".into());
    f
}

fn parse_json(data: &[u8]) -> Result<Vec<Finding>, ParserError> {
    let report: GenericReport = serde_json::from_slice(data)?;
    Ok(report.findings.into_iter().map(finding_from_item).collect())
}

fn parse_csv(data: &[u8]) -> Result<Vec<Finding>, ParserError> {
    let text = std::str::from_utf8(data).map_err(|_| ParserError::MissingField("utf8"))?;
    let mut rows = csv_rows(text);
    let header = match rows.next() {
        Some(h) => h,
        None => return Ok(Vec::new()),
    };
    let col = |name: &str| header.iter().position(|h| h == name);
    let (i_title, i_sev, i_desc) = (col("Title"), col("Severity"), col("Description"));

    // Optional columns.
    let i_cwe = col("CweId");
    let i_cve = col("CVE");
    let i_active = col("Active");
    let i_verified = col("Verified");
    let i_fp = col("FalsePositive");
    let i_dup = col("Duplicate");
    let i_mit = col("Mitigation");
    let i_impact = col("Impact");
    let i_refs = col("References");

    let get = |row: &[String], idx: Option<usize>| -> Option<String> {
        idx.and_then(|i| row.get(i)).map(|s| s.to_string())
    };

    // Dedup-by-key, preserving first-seen order.
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, Finding> = HashMap::new();

    for row in rows {
        if row.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let title = get(&row, i_title).unwrap_or_default();
        let sev = normalize_severity(&get(&row, i_sev).unwrap_or_default());
        let desc = get(&row, i_desc).unwrap_or_default();
        let key = dedupe_key(sev, &title, &desc);

        if let Some(existing) = by_key.get_mut(&key) {
            existing.nb_occurences += 1;
            continue;
        }

        let mut f = Finding::new(title, sev);
        f.description = desc;
        if let Some(c) = get(&row, i_cwe).and_then(|s| s.trim().parse::<u32>().ok()) {
            f.cwe = Some(c);
        }
        if let Some(cve) = get(&row, i_cve).filter(|s| !s.trim().is_empty()) {
            f.cve = Some(cve.clone());
            f.vulnerability_ids = vec![cve];
        }
        f.mitigation = get(&row, i_mit).filter(|s| !s.is_empty());
        f.impact = get(&row, i_impact).filter(|s| !s.is_empty());
        f.references = get(&row, i_refs).filter(|s| !s.is_empty());
        f.state.active = get(&row, i_active).map(|s| csv_bool(&s)).unwrap_or(true);
        f.state.verified = get(&row, i_verified).map(|s| csv_bool(&s)).unwrap_or(false);
        f.state.false_p = get(&row, i_fp).map(|s| csv_bool(&s)).unwrap_or(false);
        f.state.duplicate = get(&row, i_dup).map(|s| csv_bool(&s)).unwrap_or(false);
        f.found_by_scanner = Some("Generic Findings Import".into());

        order.push(key.clone());
        by_key.insert(key, f);
    }

    Ok(order.into_iter().filter_map(|k| by_key.remove(&k)).collect())
}

/// Minimal RFC-4180 reader: yields one `Vec<String>` per record, honoring
/// double-quoted fields (with `""` escaping) that may contain commas and
/// newlines. Sufficient for DefectDojo's generic CSV export.
fn csv_rows(text: &str) -> impl Iterator<Item = Vec<String>> + '_ {
    let mut chars = text.chars().peekable();
    std::iter::from_fn(move || {
        if chars.peek().is_none() {
            return None;
        }
        let mut record: Vec<String> = Vec::new();
        let mut field = String::new();
        let mut in_quotes = false;
        loop {
            match chars.next() {
                None => {
                    record.push(std::mem::take(&mut field));
                    return Some(record);
                }
                Some('"') if in_quotes => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        in_quotes = false;
                    }
                }
                Some('"') if field.is_empty() && !in_quotes => in_quotes = true,
                Some(',') if !in_quotes => record.push(std::mem::take(&mut field)),
                Some('\n') if !in_quotes => {
                    record.push(std::mem::take(&mut field));
                    return Some(record);
                }
                Some('\r') if !in_quotes => {} // swallow CR in CRLF
                Some(c) => field.push(c),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON_SAMPLE: &[u8] = br#"{
        "findings": [
            {"title": "SQL Injection", "severity": "Critical",
             "description": "SQL injection in login",
             "cve": "CVE-2024-1234", "cwe": 89,
             "cvssv3": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H",
             "cvssv3_score": 9.8, "active": true, "verified": true,
             "component_name": "auth", "component_version": "1.2.3"},
            {"title": "Verbose error", "severity": "BOGUS",
             "description": "stack trace leak"}
        ]
    }"#;

    #[test]
    fn json_parses_all_findings() {
        let out = GenericParser.parse(JSON_SAMPLE).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn json_maps_core_fields() {
        let out = GenericParser.parse(JSON_SAMPLE).unwrap();
        let f = &out[0];
        assert_eq!(f.title, "SQL Injection");
        assert_eq!(f.severity, FindingSeverity::Critical);
        assert_eq!(f.cve.as_deref(), Some("CVE-2024-1234"));
        assert_eq!(f.cwe, Some(89));
        assert_eq!(f.cvssv3_score, Some(9.8));
        assert_eq!(f.component_name.as_deref(), Some("auth"));
        assert_eq!(f.component_version.as_deref(), Some("1.2.3"));
        assert!(f.state.verified);
    }

    #[test]
    fn json_cve_seeds_vulnerability_ids() {
        let out = GenericParser.parse(JSON_SAMPLE).unwrap();
        assert_eq!(out[0].vulnerability_ids, vec!["CVE-2024-1234".to_string()]);
    }

    #[test]
    fn json_unknown_severity_degrades_to_info() {
        let out = GenericParser.parse(JSON_SAMPLE).unwrap();
        // "BOGUS" is not in the canonical set → Info.
        assert_eq!(out[1].severity, FindingSeverity::Info);
    }

    #[test]
    fn json_explicit_vulnerability_ids_preserved() {
        let s = br#"{"findings":[{"title":"x","severity":"Low","description":"d",
                      "vulnerability_ids":["CVE-2024-1","CVE-2024-2"]}]}"#;
        let out = GenericParser.parse(s).unwrap();
        assert_eq!(out[0].vulnerability_ids.len(), 2);
        assert_eq!(out[0].cve.as_deref(), Some("CVE-2024-1"));
    }

    #[test]
    fn csv_parses_header_and_rows() {
        let csv = b"Title,Severity,Description,CweId,CVE,Active,Verified\n\
                    XSS,High,Script injection,79,CVE-2024-5678,TRUE,t\n";
        let out = GenericParser.parse(csv).unwrap();
        assert_eq!(out.len(), 1);
        let f = &out[0];
        assert_eq!(f.title, "XSS");
        assert_eq!(f.severity, FindingSeverity::High);
        assert_eq!(f.cwe, Some(79));
        assert_eq!(f.cve.as_deref(), Some("CVE-2024-5678"));
        assert_eq!(f.vulnerability_ids, vec!["CVE-2024-5678".to_string()]);
        assert!(f.state.active);
        assert!(f.state.verified);
    }

    #[test]
    fn csv_boolean_rule_only_leading_t_is_true() {
        let csv = b"Title,Severity,Description,Active,Verified,FalsePositive\n\
                    A,Low,d,true,false,no\n\
                    B,Low,d2,T,yes,1\n";
        let out = GenericParser.parse(csv).unwrap();
        // row A: active true, verified false, fp false
        assert!(out[0].state.active);
        assert!(!out[0].state.verified);
        assert!(!out[0].state.false_p);
        // row B: active true (T), verified false (yes→y), fp false (1)
        assert!(out[1].state.active);
        assert!(!out[1].state.verified);
        assert!(!out[1].state.false_p);
    }

    #[test]
    fn csv_severity_case_sensitive_degrades() {
        let csv = b"Title,Severity,Description\n\
                    A,CRITICAL,d\n\
                    B,Critical,d2\n";
        let out = GenericParser.parse(csv).unwrap();
        assert_eq!(out[0].severity, FindingSeverity::Info); // CRITICAL → Info
        assert_eq!(out[1].severity, FindingSeverity::Critical);
    }

    #[test]
    fn csv_dedupes_on_severity_title_description() {
        let csv = b"Title,Severity,Description\n\
                    Dup,High,same body\n\
                    Dup,High,same body\n\
                    Other,High,same body\n";
        let out = GenericParser.parse(csv).unwrap();
        // First two collapse (same severity|title|description); third differs by title.
        assert_eq!(out.len(), 2);
        let dup = out.iter().find(|f| f.title == "Dup").unwrap();
        assert_eq!(dup.nb_occurences, 2);
    }

    #[test]
    fn csv_handles_quoted_field_with_comma() {
        let csv = b"Title,Severity,Description\n\
                    \"Inject, then exfil\",High,\"a, b, c\"\n";
        let out = GenericParser.parse(csv).unwrap();
        assert_eq!(out[0].title, "Inject, then exfil");
        assert_eq!(out[0].description, "a, b, c");
    }

    #[test]
    fn empty_json_findings_is_empty() {
        let out = GenericParser.parse(br#"{"findings":[]}"#).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn dedupe_key_matches_upstream_sha256_form() {
        // Sanity check that helper agrees with the documented upstream key.
        let key = super::dedupe_key(FindingSeverity::High, "T", "D");
        let expect = {
            let mut h = sha2::Sha256::new();
            sha2::Digest::update(&mut h, b"High|T|D");
            format!("{:x}", h.finalize())
        };
        assert_eq!(key, expect);
    }
}
