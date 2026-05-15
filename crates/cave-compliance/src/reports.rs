// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{ComplianceFramework, ComplianceReport, ControlException, Finding, FindingStatus};
use uuid::Uuid;

pub fn generate_report(framework: &ComplianceFramework, findings: &[Finding], exceptions: &[ControlException]) -> ComplianceReport {
    let excepted_control_ids: std::collections::HashSet<Uuid> = exceptions.iter()
        .filter(|e| e.expires_at.map_or(true, |exp| chrono::Utc::now() < exp))
        .map(|e| e.control_id)
        .collect();

    let relevant_findings: Vec<Finding> = findings.iter()
        .filter(|f| framework.controls.iter().any(|c| c.id == f.control_id))
        .cloned()
        .collect();

    let passed = relevant_findings.iter().filter(|f| f.status == FindingStatus::Pass).count();
    let failed = relevant_findings.iter().filter(|f| f.status == FindingStatus::Fail && !excepted_control_ids.contains(&f.control_id)).count();
    let warned = relevant_findings.iter().filter(|f| f.status == FindingStatus::Warn).count();
    let na = relevant_findings.iter().filter(|f| f.status == FindingStatus::NotApplicable).count();
    let manual = relevant_findings.iter().filter(|f| f.status == FindingStatus::Manual).count();
    let total = framework.controls.len();

    let compliance_score = if total > 0 {
        let effective_passed = passed + excepted_control_ids.len().min(failed);
        (effective_passed as f64 / total as f64) * 100.0
    } else { 100.0 };

    ComplianceReport {
        id: Uuid::new_v4(),
        name: format!("{} Compliance Report", framework.name),
        framework_id: framework.id,
        framework_name: framework.name.clone(),
        total_controls: total,
        passed,
        failed,
        warned,
        not_applicable: na,
        manual,
        compliance_score,
        findings: relevant_findings,
        generated_at: chrono::Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frameworks::cis_kubernetes_framework;

    #[test]
    fn test_generate_report_empty_findings() {
        let fw = cis_kubernetes_framework();
        let report = generate_report(&fw, &[], &[]);
        assert_eq!(report.passed, 0);
        assert_eq!(report.total_controls, fw.controls.len());
        assert_eq!(report.compliance_score, 0.0);
    }

    #[test]
    fn test_generate_report_all_pass() {
        let fw = cis_kubernetes_framework();
        let findings: Vec<Finding> = fw.controls.iter().map(|c| Finding {
            id: Uuid::new_v4(),
            control_id: c.id,
            control_ref: c.control_id.clone(),
            status: FindingStatus::Pass,
            target: "cluster".to_string(),
            details: "OK".to_string(),
            remediation: None,
            evidence_ids: vec![],
            checked_at: chrono::Utc::now(),
            exception_id: None,
        }).collect();
        let report = generate_report(&fw, &findings, &[]);
        assert_eq!(report.passed, fw.controls.len());
        assert!((report.compliance_score - 100.0).abs() < 0.01);
    }
}
