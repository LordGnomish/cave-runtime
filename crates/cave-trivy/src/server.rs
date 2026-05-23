// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Client/server scan mode.
//!
//! Mirrors trivy's `pkg/rpc/server` for the JSON-over-HTTP surface
//! cave-trivy exposes via `routes::create_router`. The wire types are
//! defined here so cavectl can construct requests and parse responses
//! without depending on internal scanners.

use crate::models::Report;
use crate::severity::Severity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanRequest {
    pub target: ScanTarget,
    pub artifact_name: String,
    #[serde(default)]
    pub min_severity: Option<Severity>,
    #[serde(default)]
    pub only_fixed: bool,
    #[serde(default)]
    pub format: ReportFormat,
    #[serde(default)]
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScanTarget {
    Image,
    Fs,
    Repo,
    K8s,
    Sbom,
    Secret,
    Config,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReportFormat {
    #[default]
    Json,
    Table,
    Sarif,
    Template,
    CycloneDx,
    Spdx,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanResponse {
    pub report: Report,
    pub format: ReportFormat,
    pub rendered: String,
    pub duration_ms: u128,
}

/// Mock server entry point for testing — actual axum wiring is in `routes`.
pub fn handle(req: &ScanRequest) -> ScanResponse {
    let start = std::time::Instant::now();
    let db = crate::vulndb::VulnDb::cave_default();
    let mut report = Report::new(&req.artifact_name, format!("{:?}", req.target).as_str());

    match req.target {
        ScanTarget::Sbom => {
            if let Some(text) = req.body.as_str() {
                if let Ok(r) = crate::scan_sbom::scan_sbom(&req.artifact_name, text, &db) {
                    report = r;
                }
            }
        }
        ScanTarget::Image => {
            report = Report::new(&req.artifact_name, "container_image");
        }
        ScanTarget::Fs => {
            report = Report::new(&req.artifact_name, "filesystem");
        }
        _ => {}
    }

    if let Some(s) = req.min_severity {
        let f = crate::filter::Filter::default().min_severity(s);
        f.apply(&mut report);
    }

    let rendered = match req.format {
        ReportFormat::Json => crate::report_json::write(&report).unwrap_or_default(),
        ReportFormat::Table => crate::report_table::write(&report),
        ReportFormat::Sarif => crate::report_sarif::write(&report).unwrap_or_default(),
        ReportFormat::Template => String::new(),
        ReportFormat::CycloneDx => {
            crate::sbom_cyclonedx::emit_from_report(&report).unwrap_or_default()
        }
        ReportFormat::Spdx => crate::sbom_spdx::emit(&req.artifact_name, &[]).unwrap_or_default(),
    };
    ScanResponse {
        report,
        format: req.format,
        rendered,
        duration_ms: start.elapsed().as_millis(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_json() {
        let req = ScanRequest {
            target: ScanTarget::Image,
            artifact_name: "alpine:3.19".into(),
            min_severity: Some(Severity::High),
            only_fixed: false,
            format: ReportFormat::Json,
            body: serde_json::Value::Null,
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: ScanRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn handle_sbom_runs() {
        let sbom = r#"{"bomFormat":"CycloneDX","components":[
            {"name":"openssl-sys","version":"0.9.0","purl":"pkg:cargo/openssl-sys@0.9.0"}
        ]}"#;
        let req = ScanRequest {
            target: ScanTarget::Sbom,
            artifact_name: "x".into(),
            min_severity: None,
            only_fixed: false,
            format: ReportFormat::Json,
            body: serde_json::Value::String(sbom.into()),
        };
        let resp = handle(&req);
        assert!(resp.report.total_vulns() >= 1);
        assert!(resp.rendered.contains("CVE-2026-0030"));
    }

    #[test]
    fn handle_filter_applied() {
        let req = ScanRequest {
            target: ScanTarget::Image,
            artifact_name: "x".into(),
            min_severity: Some(Severity::Critical),
            only_fixed: true,
            format: ReportFormat::Table,
            body: serde_json::Value::Null,
        };
        let resp = handle(&req);
        assert!(resp.rendered.contains("Total vulnerabilities: 0"));
    }

    #[test]
    fn report_format_default_is_json() {
        let req = ScanRequest {
            target: ScanTarget::Image,
            artifact_name: "x".into(),
            min_severity: None,
            only_fixed: false,
            format: ReportFormat::default(),
            body: serde_json::Value::Null,
        };
        assert_eq!(req.format, ReportFormat::Json);
    }
}
