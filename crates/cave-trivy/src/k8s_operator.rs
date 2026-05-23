// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Trivy-Operator CRD shapes (VulnerabilityReport, ConfigAuditReport,
//! SbomReport, ExposedSecretReport).
//!
//! Mirrors aquasecurity/trivy-operator CRDs. cave-trivy emits the
//! per-workload report-shaped values; the actual controller-reconciliation
//! loop is delegated to cave-controller-manager.

use crate::models::Report;
use crate::severity::Severity;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VulnerabilityReport {
    pub api_version: &'static str,
    pub kind: &'static str,
    pub metadata: ObjectMeta,
    pub report: VulnerabilityReportData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMeta {
    pub name: String,
    pub namespace: String,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VulnerabilityReportData {
    pub updated_at: String,
    pub scanner: ScannerInfo,
    pub registry: String,
    pub artifact: ArtifactInfo,
    pub summary: SeverityCount,
    pub vulnerabilities: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannerInfo {
    pub name: String,
    pub vendor: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactInfo {
    pub repository: String,
    pub tag: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SeverityCount {
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub unknown: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub vulnerability_id: String,
    pub resource: String,
    pub installed_version: String,
    pub fixed_version: Option<String>,
    pub severity: Severity,
}

impl VulnerabilityReport {
    pub fn from_scan(
        name: &str,
        namespace: &str,
        repo: &str,
        tag: &str,
        digest: &str,
        report: &Report,
    ) -> Self {
        let mut summary = SeverityCount::default();
        let mut findings = Vec::new();
        for r in &report.results {
            for v in &r.vulnerabilities {
                match v.severity {
                    Severity::Critical => summary.critical += 1,
                    Severity::High => summary.high += 1,
                    Severity::Medium => summary.medium += 1,
                    Severity::Low => summary.low += 1,
                    Severity::Unknown => summary.unknown += 1,
                }
                findings.push(Finding {
                    vulnerability_id: v.id.clone(),
                    resource: v.pkg_name.clone(),
                    installed_version: v.installed_version.clone(),
                    fixed_version: v.fixed_version.clone(),
                    severity: v.severity,
                });
            }
        }
        Self {
            api_version: "aquasecurity.github.io/v1alpha1",
            kind: "VulnerabilityReport",
            metadata: ObjectMeta {
                name: name.into(),
                namespace: namespace.into(),
                labels: HashMap::new(),
            },
            report: VulnerabilityReportData {
                updated_at: chrono::Utc::now().to_rfc3339(),
                scanner: ScannerInfo {
                    name: "cave-trivy".into(),
                    vendor: "cave-runtime".into(),
                    version: crate::UPSTREAM_VERSION.into(),
                },
                registry: registry_of(repo),
                artifact: ArtifactInfo {
                    repository: repo.into(),
                    tag: tag.into(),
                    digest: digest.into(),
                },
                summary,
                vulnerabilities: findings,
            },
        }
    }
}

fn registry_of(repo: &str) -> String {
    repo.split_once('/')
        .map(|(host, _)| host.to_string())
        .unwrap_or_else(|| "docker.io".into())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigAuditReport {
    pub api_version: &'static str,
    pub kind: &'static str,
    pub metadata: ObjectMeta,
    pub checks: Vec<ConfigCheck>,
    pub summary: SeverityCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigCheck {
    pub check_id: String,
    pub title: String,
    pub severity: Severity,
    pub success: bool,
    pub category: String,
}

impl ConfigAuditReport {
    pub fn from_report(name: &str, namespace: &str, report: &Report) -> Self {
        let mut summary = SeverityCount::default();
        let mut checks = Vec::new();
        for r in &report.results {
            for m in &r.misconfigurations {
                match m.severity {
                    Severity::Critical => summary.critical += 1,
                    Severity::High => summary.high += 1,
                    Severity::Medium => summary.medium += 1,
                    Severity::Low => summary.low += 1,
                    Severity::Unknown => summary.unknown += 1,
                }
                checks.push(ConfigCheck {
                    check_id: m.id.clone(),
                    title: m.title.clone(),
                    severity: m.severity,
                    success: false,
                    category: m.r#type.clone(),
                });
            }
        }
        Self {
            api_version: "aquasecurity.github.io/v1alpha1",
            kind: "ConfigAuditReport",
            metadata: ObjectMeta {
                name: name.into(),
                namespace: namespace.into(),
                labels: HashMap::new(),
            },
            checks,
            summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Report, ScanResult, Vulnerability};

    #[test]
    fn vuln_report_summary_counts() {
        let mut report = Report::new("app", "container_image");
        let mut sr = ScanResult::default();
        sr.vulnerabilities.push(Vulnerability::new("X", "p", "1", Severity::Critical));
        sr.vulnerabilities.push(Vulnerability::new("Y", "q", "1", Severity::High));
        sr.vulnerabilities.push(Vulnerability::new("Z", "r", "1", Severity::Medium));
        report.results.push(sr);
        let r = VulnerabilityReport::from_scan("app", "ns", "ghcr.io/cave/app", "1", "sha:1", &report);
        assert_eq!(r.report.summary.critical, 1);
        assert_eq!(r.report.summary.high, 1);
        assert_eq!(r.report.summary.medium, 1);
        assert_eq!(r.report.scanner.name, "cave-trivy");
    }

    #[test]
    fn registry_extracted() {
        let r = registry_of("ghcr.io/cave/app");
        assert_eq!(r, "ghcr.io");
        let r2 = registry_of("nginx");
        assert_eq!(r2, "docker.io");
    }

    #[test]
    fn config_audit_report() {
        let mut report = Report::new("app", "k8s_cluster");
        report.results.push(ScanResult {
            target: "Pod/ns/p".into(),
            class: "config".into(),
            misconfigurations: vec![crate::models::Misconfiguration {
                id: "AVD-KSV-0017".into(),
                r#type: "kubernetes".into(),
                title: "Privileged".into(),
                description: "no".into(),
                severity: Severity::Critical,
                resource: "Pod/p".into(),
                references: vec![],
            }],
            ..Default::default()
        });
        let r = ConfigAuditReport::from_report("p", "ns", &report);
        assert_eq!(r.checks.len(), 1);
        assert_eq!(r.summary.critical, 1);
        assert_eq!(r.kind, "ConfigAuditReport");
    }

    #[test]
    fn vuln_report_serialises() {
        let report = Report::new("x", "container_image");
        let r = VulnerabilityReport::from_scan("x", "ns", "ghcr.io/x", "1", "sha:1", &report);
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("VulnerabilityReport"));
    }
}
