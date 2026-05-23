// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! SARIF 2.1.0 report writer.
//!
//! Mirrors trivy's `pkg/report/sarif`. Each `Vulnerability` /
//! `Misconfiguration` / `Secret` becomes one SARIF `Result` under a
//! `runs[0]` block; rules are deduplicated into `runs[0].tool.driver
//! .rules`. The cave-trivy SARIF stays minimal — rule helpUri, severity
//! → `level`, locations → physicalLocation/artifactLocation.

use crate::error::{TrivyError, TrivyResult};
use crate::models::Report;
use crate::severity::Severity;
use serde_json::json;
use std::collections::HashSet;

pub fn write(report: &Report) -> TrivyResult<String> {
    let mut rules = HashSet::new();
    let mut results = Vec::new();
    let mut rule_defs = Vec::new();

    for r in &report.results {
        for v in &r.vulnerabilities {
            if rules.insert(v.id.clone()) {
                rule_defs.push(json!({
                    "id": v.id,
                    "name": v.id,
                    "shortDescription": { "text": v.title.clone().unwrap_or_else(|| v.id.clone()) },
                    "helpUri": v.references.first().cloned().unwrap_or_default(),
                }));
            }
            results.push(json!({
                "ruleId": v.id,
                "level": severity_to_level(v.severity),
                "message": { "text": format!("{} affects {} {}", v.id, v.pkg_name, v.installed_version) },
                "locations": [{ "physicalLocation": { "artifactLocation": { "uri": r.target } } }],
            }));
        }
        for m in &r.misconfigurations {
            if rules.insert(m.id.clone()) {
                rule_defs.push(json!({
                    "id": m.id,
                    "name": m.id,
                    "shortDescription": { "text": m.title },
                    "helpUri": m.references.first().cloned().unwrap_or_default(),
                }));
            }
            results.push(json!({
                "ruleId": m.id,
                "level": severity_to_level(m.severity),
                "message": { "text": m.description },
                "locations": [{ "physicalLocation": { "artifactLocation": { "uri": m.resource } } }],
            }));
        }
        for s in &r.secrets {
            if rules.insert(s.rule_id.clone()) {
                rule_defs.push(json!({
                    "id": s.rule_id,
                    "name": s.rule_id,
                    "shortDescription": { "text": s.category },
                }));
            }
            results.push(json!({
                "ruleId": s.rule_id,
                "level": severity_to_level(s.severity),
                "message": { "text": format!("secret detected in {}", s.file) },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": s.file },
                        "region": { "startLine": s.start_line, "endLine": s.end_line }
                    }
                }],
            }));
        }
    }

    let doc = json!({
        "$schema": "https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0-rtm.5.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "cave-trivy",
                "version": crate::UPSTREAM_VERSION,
                "informationUri": "https://github.com/cave-runtime/cave-runtime",
                "rules": rule_defs,
            }},
            "results": results,
        }]
    });
    serde_json::to_string_pretty(&doc).map_err(|e| TrivyError::Report(format!("sarif: {}", e)))
}

fn severity_to_level(s: Severity) -> &'static str {
    match s {
        Severity::Critical | Severity::High => "error",
        Severity::Medium => "warning",
        Severity::Low | Severity::Unknown => "note",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Report, ScanResult, Vulnerability};
    use crate::severity::Severity;

    #[test]
    fn empty_report_valid_sarif() {
        let r = Report::new("x", "y");
        let j = write(&r).unwrap();
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["version"], "2.1.0");
        assert_eq!(v["runs"][0]["tool"]["driver"]["name"], "cave-trivy");
    }

    #[test]
    fn vulns_emit_results() {
        let mut r = Report::new("img", "container_image");
        r.results.push(ScanResult {
            target: "img".into(),
            class: "os".into(),
            vulnerabilities: vec![Vulnerability {
                id: "CVE-1".into(),
                pkg_name: "p".into(),
                installed_version: "1".into(),
                fixed_version: None,
                severity: Severity::Critical,
                references: vec!["https://example/1".into()],
                title: Some("xx".into()),
            }],
            ..Default::default()
        });
        let j = write(&r).unwrap();
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let results = v["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["ruleId"], "CVE-1");
        assert_eq!(results[0]["level"], "error");
    }

    #[test]
    fn severity_to_level_mapping() {
        assert_eq!(severity_to_level(Severity::Critical), "error");
        assert_eq!(severity_to_level(Severity::High), "error");
        assert_eq!(severity_to_level(Severity::Medium), "warning");
        assert_eq!(severity_to_level(Severity::Low), "note");
        assert_eq!(severity_to_level(Severity::Unknown), "note");
    }

    #[test]
    fn rule_dedup_across_results() {
        let mut r = Report::new("x", "y");
        for _ in 0..3 {
            r.results.push(ScanResult {
                target: "x".into(),
                class: "os".into(),
                vulnerabilities: vec![Vulnerability {
                    id: "CVE-DUP".into(),
                    pkg_name: "p".into(),
                    installed_version: "1".into(),
                    fixed_version: None,
                    severity: Severity::High,
                    references: vec![],
                    title: None,
                }],
                ..Default::default()
            });
        }
        let j = write(&r).unwrap();
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let rules = v["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap();
        assert_eq!(rules.len(), 1);
    }
}
