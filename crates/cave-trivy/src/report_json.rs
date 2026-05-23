// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! JSON report writer.
//!
//! Mirrors trivy's `pkg/report/writer.JSONWriter`. The report serialises
//! straight from `Report` via serde with `SchemaVersion` at the top.

use crate::error::{TrivyError, TrivyResult};
use crate::models::Report;

pub fn write(report: &Report) -> TrivyResult<String> {
    serde_json::to_string_pretty(report).map_err(|e| TrivyError::Report(format!("json: {}", e)))
}

pub fn parse(text: &str) -> TrivyResult<Report> {
    serde_json::from_str(text).map_err(|e| TrivyError::Report(format!("json parse: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Report, ScanResult, Vulnerability};
    use crate::severity::Severity;

    #[test]
    fn round_trip() {
        let mut r = Report::new("img", "container_image");
        r.results.push(ScanResult {
            target: "img".into(),
            class: "os".into(),
            vulnerabilities: vec![Vulnerability {
                id: "CVE-1".into(),
                pkg_name: "p".into(),
                installed_version: "1".into(),
                fixed_version: Some("2".into()),
                severity: Severity::High,
                references: vec![],
                title: None,
            }],
            ..Default::default()
        });
        let j = write(&r).unwrap();
        let back = parse(&j).unwrap();
        assert_eq!(back.artifact_name, "img");
        assert_eq!(back.total_vulns(), 1);
    }

    #[test]
    fn parse_bad_json() {
        assert!(parse("not json").is_err());
    }

    #[test]
    fn json_top_level_schema_version() {
        let r = Report::new("x", "y");
        let j = write(&r).unwrap();
        assert!(j.contains("\"schema_version\": 2"));
    }
}
