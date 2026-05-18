// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{Finding, ScanKind, ScanRequest, ScanResult, ScanStatus, ScanVerdict, Severity, VerdictDecision};
use chrono::Utc;
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum ScanError {
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Scanner unavailable for {0}")]
    ScannerUnavailable(ScanKind),

    #[error("Scan timeout")]
    Timeout,

    #[error("Upstream fetch failed: {0}")]
    UpstreamFetchFailed(String),

    #[error("Parse failure: {0}")]
    ParseFailure(String),
}

// ---------------------------------------------------------------------------
// Scanner trait
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait Scanner: Send + Sync {
    fn kind(&self) -> ScanKind;
    async fn scan(&self, req: &ScanRequest) -> Result<Vec<Finding>, ScanError>;
}

// ---------------------------------------------------------------------------
// Deduplication and verdict logic
// ---------------------------------------------------------------------------

pub fn dedupe_findings(findings: Vec<Finding>) -> Vec<Finding> {
    let mut seen = HashMap::new();
    let mut deduped = Vec::new();

    for finding in findings {
        let key = finding.fingerprint.clone();
        if !seen.contains_key(&key) {
            seen.insert(key, true);
            deduped.push(finding);
        }
    }

    deduped
}

pub fn aggregate_verdict(findings: &[Finding], floor: Option<Severity>) -> ScanVerdict {
    let floor = floor.unwrap_or(Severity::Low);

    // Check severity levels
    let has_critical_or_high = findings.iter().any(|f| f.severity == Severity::Critical || f.severity == Severity::High);
    let has_medium = findings.iter().any(|f| f.severity == Severity::Medium);

    let decision = if has_critical_or_high {
        VerdictDecision::Fail
    } else if has_medium {
        VerdictDecision::Warn
    } else {
        VerdictDecision::Pass
    };

    let finding_ids: Vec<Uuid> = findings.iter().map(|f| f.id).collect();
    let reasons = match decision {
        VerdictDecision::Fail => {
            let critical = findings.iter().filter(|f| f.severity == Severity::Critical).count();
            let high = findings.iter().filter(|f| f.severity == Severity::High).count();
            vec![format!("Found {} critical and {} high severity issues", critical, high)]
        }
        VerdictDecision::Warn => {
            vec!["Found medium severity issues".to_string()]
        }
        VerdictDecision::Pass => {
            vec!["All findings below threshold or no findings".to_string()]
        }
    };

    ScanVerdict {
        decision,
        reasons,
        finding_ids,
        evaluated_at: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

pub struct ScanOrchestrator {
    scanners: Vec<Box<dyn Scanner>>,
}

impl ScanOrchestrator {
    pub fn new(scanners: Vec<Box<dyn Scanner>>) -> Self {
        Self { scanners }
    }

    pub async fn run(&self, req: &ScanRequest) -> ScanResult {
        let started_at = Utc::now();
        let id = Uuid::new_v4();

        let scanner = self.scanners.iter().find(|s| s.kind() == req.kind);

        let (findings, status) = match scanner {
            Some(s) => {
                match s.scan(req).await {
                    Ok(findings) => {
                        let deduped = dedupe_findings(findings);
                        (deduped, ScanStatus::Completed)
                    }
                    Err(_e) => (vec![], ScanStatus::Failed),
                }
            }
            None => (vec![], ScanStatus::Failed),
        };

        let finished_at = Utc::now();

        ScanResult {
            id,
            request: req.clone(),
            findings,
            started_at,
            finished_at,
            scanner_version: "0.1.0".to_string(),
            status,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FindingCategory, Confidence};

    fn sample_finding(severity: Severity) -> Finding {
        let mut f = Finding::new(
            "TEST-001".to_string(),
            "Test".to_string(),
            FindingCategory::Misconfig,
            severity,
            "Test".to_string(),
            "Test finding".to_string(),
        );
        f.confidence = Confidence::High;
        f
    }

    #[test]
    fn test_dedupe_findings() {
        let f1 = sample_finding(Severity::High);
        let f2 = sample_finding(Severity::High);

        let mut findings = vec![f1, f2];
        findings[1].id = Uuid::new_v4();

        let deduped = dedupe_findings(findings);
        assert_eq!(deduped.len(), 1);
    }

    #[test]
    fn test_aggregate_verdict_fail() {
        let findings = vec![sample_finding(Severity::Critical), sample_finding(Severity::Medium)];
        let verdict = aggregate_verdict(&findings, None);
        assert_eq!(verdict.decision, VerdictDecision::Fail);
        assert!(verdict.reasons[0].contains("critical"));
    }

    #[test]
    fn test_aggregate_verdict_warn() {
        let findings = vec![sample_finding(Severity::Medium), sample_finding(Severity::Low)];
        let verdict = aggregate_verdict(&findings, None);
        assert_eq!(verdict.decision, VerdictDecision::Warn);
    }

    #[test]
    fn test_aggregate_verdict_pass() {
        let findings = vec![sample_finding(Severity::Low), sample_finding(Severity::Info)];
        let verdict = aggregate_verdict(&findings, None);
        assert_eq!(verdict.decision, VerdictDecision::Pass);
    }

    #[test]
    fn test_aggregate_verdict_no_findings() {
        let findings = vec![];
        let verdict = aggregate_verdict(&findings, None);
        assert_eq!(verdict.decision, VerdictDecision::Pass);
    }
}
