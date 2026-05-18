// SPDX-License-Identifier: AGPL-3.0-or-later
//! SARIF v2.1.0 parser.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/tools/sarif/parser.py
//!         (`class SarifParser`). OASIS spec:
//!         <https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html>
//!
//! Maps:
//!   - `runs[].results[].ruleId / message.text` → title/description
//!   - `runs[].results[].level` (`error`/`warning`/`note`/`none`) → severity
//!   - `runs[].results[].locations[0].physicalLocation` → file_path/line
//!   - `runs[].results[].properties.security-severity` → cvssv3_score
//!   - `runs[].tool.driver.rules[].properties.tags` cwe-N → cwe

use super::{ParserError, ScanParser};
use crate::cvss::severity_from_score;
use crate::finding::{Finding, FindingSeverity};
use serde::Deserialize;
use serde_json::Value;

pub struct SarifParser;

#[derive(Deserialize)]
struct SarifReport {
    #[serde(default)]
    runs: Vec<Run>,
}
#[derive(Deserialize)]
struct Run {
    #[serde(default)]
    results: Vec<SrResult>,
    #[serde(default)]
    tool: Tool,
}
#[derive(Deserialize, Default)]
struct Tool {
    #[serde(default)]
    driver: Driver,
}
#[derive(Deserialize, Default)]
struct Driver {
    #[serde(default)]
    name: String,
    #[serde(default)]
    rules: Vec<Rule>,
}
#[derive(Deserialize)]
struct Rule {
    id: String,
    #[serde(default)]
    properties: Value,
    #[serde(default, rename = "shortDescription")]
    short_description: Value,
    #[serde(default, rename = "fullDescription")]
    #[allow(dead_code)]
    full_description: Value,
}
#[derive(Deserialize)]
struct SrResult {
    #[serde(default, rename = "ruleId")]
    rule_id: Option<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    message: Value,
    #[serde(default)]
    locations: Vec<Location>,
    #[serde(default)]
    properties: Value,
    #[serde(default, rename = "partialFingerprints")]
    partial_fingerprints: Value,
    #[serde(default)]
    suppressions: Vec<Value>,
}
#[derive(Deserialize)]
struct Location {
    #[serde(default, rename = "physicalLocation")]
    physical: Value,
}

fn level_to_severity(level: &Option<String>, sec_severity: Option<f32>) -> FindingSeverity {
    // GHAS CodeQL pattern: prefer security-severity numeric if present.
    if let Some(s) = sec_severity {
        return severity_from_score(s);
    }
    match level.as_deref().map(|l| l.to_ascii_lowercase()) {
        Some(s) if s == "error" => FindingSeverity::High,
        Some(s) if s == "warning" => FindingSeverity::Medium,
        Some(s) if s == "note" => FindingSeverity::Low,
        _ => FindingSeverity::Info, // `none` or unspecified.
    }
}

fn extract_cwe(rule_props: &Value) -> Option<u32> {
    // SARIF carries CWE as a tag like `external/cwe/cwe-79`.
    if let Some(tags) = rule_props.get("tags").and_then(|v| v.as_array()) {
        for t in tags {
            if let Some(s) = t.as_str() {
                let lower = s.to_ascii_lowercase();
                if let Some(idx) = lower.rfind("cwe-") {
                    if let Some(num) = lower[idx + 4..]
                        .split(|c: char| !c.is_ascii_digit())
                        .next()
                    {
                        if let Ok(n) = num.parse::<u32>() {
                            return Some(n);
                        }
                    }
                }
            }
        }
    }
    None
}

impl ScanParser for SarifParser {
    fn scan_type(&self) -> &'static str { "SARIF" }
    fn dedupe_fields(&self) -> &'static [&'static str] {
        &["title", "cwe", "line", "file_path", "description"]
    }
    fn parse(&self, data: &[u8]) -> Result<Vec<Finding>, ParserError> {
        let report: SarifReport = serde_json::from_slice(data)?;
        let mut out = Vec::new();
        for run in report.runs {
            let driver_name = run.tool.driver.name.clone();
            for r in run.results {
                let rule_id = r.rule_id.clone().unwrap_or_else(|| "(unknown)".into());
                // Look up the rule by id to harvest CWE + descriptions.
                let rule = run.tool.driver.rules.iter().find(|x| x.id == rule_id);
                let cwe = rule.and_then(|x| extract_cwe(&x.properties));
                let sec_severity = r.properties.get("security-severity")
                    .and_then(|v| v.as_str().and_then(|s| s.parse::<f32>().ok())
                        .or_else(|| v.as_f64().map(|x| x as f32)));
                let sev = level_to_severity(&r.level, sec_severity);
                let title = if let Some(rl) = rule {
                    rl.short_description.get("text").and_then(|v| v.as_str()).map(String::from)
                        .unwrap_or_else(|| rule_id.clone())
                } else {
                    rule_id.clone()
                };
                let mut f = Finding::new(title, sev);
                f.cwe = cwe;
                f.cvssv3_score = sec_severity;
                f.vuln_id_from_tool = Some(rule_id);
                f.found_by_scanner = Some("SARIF".into());
                if !driver_name.is_empty() {
                    f.service = Some(driver_name.clone());
                }
                let msg = r.message.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !msg.is_empty() {
                    f.description = msg;
                }
                if let Some(loc) = r.locations.first() {
                    if let Some(art) = loc.physical.get("artifactLocation").and_then(|a| a.get("uri")).and_then(|v| v.as_str()) {
                        f.file_path = Some(art.into());
                    }
                    if let Some(region) = loc.physical.get("region") {
                        if let Some(l) = region.get("startLine").and_then(|v| v.as_u64()) {
                            f.line = Some(l as u32);
                        }
                    }
                }
                if let Some(fp) = r.partial_fingerprints.as_object().and_then(|o| o.values().next()).and_then(|v| v.as_str()) {
                    f.unique_id_from_tool = Some(fp.into());
                }
                if !r.suppressions.is_empty() {
                    // Suppressed → false_positive flag (DefectDojo mirrors `false_p`).
                    f.state.false_p = true;
                    f.state.active = false;
                }
                f.static_finding = true;
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
      "version": "2.1.0",
      "runs": [{
        "tool": { "driver": { "name": "CodeQL", "rules": [
          {"id":"py/sql-injection","properties":{"tags":["security","external/cwe/cwe-89"]},
           "shortDescription":{"text":"SQL Injection"}}
        ]}},
        "results": [
          {"ruleId":"py/sql-injection","level":"error",
           "message":{"text":"User input flows to SQL"},
           "locations":[{"physicalLocation":{"artifactLocation":{"uri":"app/db.py"},
                                              "region":{"startLine":42}}}],
           "properties":{"security-severity":"9.5"},
           "partialFingerprints":{"primaryLocationLineHash":"abc"}},
          {"ruleId":"py/info","level":"note",
           "message":{"text":"informational"}},
          {"ruleId":"py/warn","level":"warning",
           "message":{"text":"warn"},
           "suppressions":[{"kind":"external"}]}
        ]
      }]
    }"#;

    #[test]
    fn parses_three_results() {
        let out = SarifParser.parse(SAMPLE).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn security_severity_promotes_to_critical() {
        let out = SarifParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].severity, FindingSeverity::Critical);
        assert_eq!(out[0].cvssv3_score, Some(9.5));
    }

    #[test]
    fn level_note_maps_to_low() {
        let out = SarifParser.parse(SAMPLE).unwrap();
        assert_eq!(out[1].severity, FindingSeverity::Low);
    }

    #[test]
    fn level_warning_maps_to_medium() {
        let out = SarifParser.parse(SAMPLE).unwrap();
        assert_eq!(out[2].severity, FindingSeverity::Medium);
    }

    #[test]
    fn extracts_cwe_from_tags() {
        let out = SarifParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].cwe, Some(89));
    }

    #[test]
    fn extracts_file_path_and_line() {
        let out = SarifParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].file_path.as_deref(), Some("app/db.py"));
        assert_eq!(out[0].line, Some(42));
    }

    #[test]
    fn extracts_fingerprint_as_unique_id() {
        let out = SarifParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].unique_id_from_tool.as_deref(), Some("abc"));
    }

    #[test]
    fn suppression_marks_false_positive() {
        let out = SarifParser.parse(SAMPLE).unwrap();
        assert!(out[2].state.false_p);
        assert!(!out[2].state.active);
    }

    #[test]
    fn driver_name_lands_on_service() {
        let out = SarifParser.parse(SAMPLE).unwrap();
        assert_eq!(out[0].service.as_deref(), Some("CodeQL"));
    }

    #[test]
    fn empty_runs_returns_empty() {
        let out = SarifParser.parse(br#"{"version":"2.1.0","runs":[]}"#).unwrap();
        assert!(out.is_empty());
    }
}
