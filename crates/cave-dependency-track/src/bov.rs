// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Bill of Vulnerabilities (BOV) export — CycloneDX-VDR variant.
//!
//! Upstream: `model/Finding` + `resources/v1/FindingResource` BOM emit
//! path; "vulnerability disclosure report" sub-section in the CycloneDX
//! 1.6 spec.

use crate::audit::AuditStore;
use crate::models::{Severity, Vulnerability};
use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct BovDocument {
    pub project: Uuid,
    pub generated_at: chrono::DateTime<Utc>,
    pub findings: Vec<Value>,
    pub summary: BovSummary,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BovSummary {
    pub total: usize,
    pub critical: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub info: usize,
    pub unassigned: usize,
    pub suppressed: usize,
}

impl BovDocument {
    pub fn build(
        project: Uuid,
        component_to_vulns: &[(Uuid, Vec<Vulnerability>)],
        audit: &AuditStore,
    ) -> Self {
        let mut findings = Vec::new();
        let mut summary = BovSummary::default();
        for (component, vulns) in component_to_vulns {
            for v in vulns {
                let is_suppressed = audit.is_suppressed(*component, v.uuid);
                if is_suppressed {
                    summary.suppressed += 1;
                    continue;
                }
                summary.total += 1;
                match v.severity {
                    Severity::Critical => summary.critical += 1,
                    Severity::High => summary.high += 1,
                    Severity::Medium => summary.medium += 1,
                    Severity::Low => summary.low += 1,
                    Severity::Info => summary.info += 1,
                    Severity::Unassigned => summary.unassigned += 1,
                }
                findings.push(json!({
                    "component": component.to_string(),
                    "vulnerability": {
                        "id": v.vuln_id,
                        "source": format!("{:?}", v.source).to_uppercase(),
                        "severity": format!("{:?}", v.severity).to_uppercase(),
                        "cvssV3BaseScore": v.cvss_v3_base_score,
                        "epssScore": v.epss_score,
                        "epssPercentile": v.epss_percentile,
                        "cwes": v.cwes,
                    }
                }));
            }
        }
        Self {
            project,
            generated_at: Utc::now(),
            findings,
            summary,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "project": self.project.to_string(),
            "generatedAt": self.generated_at.to_rfc3339(),
            "summary": {
                "total": self.summary.total,
                "suppressed": self.summary.suppressed,
                "critical": self.summary.critical,
                "high": self.summary.high,
                "medium": self.summary.medium,
                "low": self.summary.low,
                "info": self.summary.info,
                "unassigned": self.summary.unassigned,
            },
            "findings": self.findings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AnalysisState, VulnSource};

    fn v(id: &str, s: Severity) -> Vulnerability {
        let mut x = Vulnerability::new(id, VulnSource::Nvd);
        x.severity = s;
        x
    }

    #[test]
    fn summary_counts_by_severity() {
        let audit = AuditStore::new();
        let comp = Uuid::new_v4();
        let inputs = vec![(
            comp,
            vec![
                v("CVE-1", Severity::Critical),
                v("CVE-2", Severity::High),
                v("CVE-3", Severity::Medium),
            ],
        )];
        let bov = BovDocument::build(Uuid::new_v4(), &inputs, &audit);
        assert_eq!(bov.summary.total, 3);
        assert_eq!(bov.summary.critical, 1);
        assert_eq!(bov.summary.high, 1);
        assert_eq!(bov.summary.medium, 1);
    }

    #[test]
    fn suppressed_excluded_from_findings_but_counted() {
        let audit = AuditStore::new();
        let comp = Uuid::new_v4();
        let v = v("CVE-X", Severity::High);
        let vuln_uuid = v.uuid;
        let inputs = vec![(comp, vec![v])];
        audit.upsert(comp, vuln_uuid, AnalysisState::FalsePositive);
        let bov = BovDocument::build(Uuid::new_v4(), &inputs, &audit);
        assert_eq!(bov.summary.total, 0);
        assert_eq!(bov.summary.suppressed, 1);
        assert!(bov.findings.is_empty());
    }

    #[test]
    fn empty_input_zero_summary() {
        let bov = BovDocument::build(Uuid::new_v4(), &[], &AuditStore::new());
        assert_eq!(bov.summary.total, 0);
        assert!(bov.findings.is_empty());
    }

    #[test]
    fn to_json_includes_summary_and_findings() {
        let audit = AuditStore::new();
        let inputs = vec![(Uuid::new_v4(), vec![v("CVE-1", Severity::Low)])];
        let bov = BovDocument::build(Uuid::new_v4(), &inputs, &audit);
        let j = bov.to_json();
        assert!(j.get("summary").is_some());
        assert_eq!(j["findings"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn unassigned_severity_bucketed() {
        let audit = AuditStore::new();
        let inputs = vec![(Uuid::new_v4(), vec![v("CVE-1", Severity::Unassigned)])];
        let bov = BovDocument::build(Uuid::new_v4(), &inputs, &audit);
        assert_eq!(bov.summary.unassigned, 1);
    }
}
