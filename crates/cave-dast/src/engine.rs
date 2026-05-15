// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{DastFinding, DastScan, RiskLevel};
use std::collections::HashMap;

pub fn risk_rank(risk: &RiskLevel) -> u8 {
    match risk {
        RiskLevel::Informational => 0,
        RiskLevel::Low => 1,
        RiskLevel::Medium => 2,
        RiskLevel::High => 3,
    }
}

pub fn count_by_risk(findings: &[DastFinding]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for f in findings {
        let key = format!("{:?}", f.risk);
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

pub fn high_risk_findings(findings: &[DastFinding]) -> Vec<&DastFinding> {
    findings.iter().filter(|f| f.risk == RiskLevel::High).collect()
}

pub fn scan_score(scan: &DastScan) -> u32 {
    scan.findings.iter().map(|f| risk_rank(&f.risk) as u32).sum()
}

pub fn is_valid_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

pub fn findings_with_cwe(findings: &[DastFinding]) -> Vec<&DastFinding> {
    findings.iter().filter(|f| f.cwe_id.is_some()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DastScan, ScanStatus, ScanType};
    use uuid::Uuid;
    use chrono::Utc;

    fn make_finding(risk: RiskLevel, cwe_id: Option<u32>) -> DastFinding {
        DastFinding {
            id: Uuid::new_v4(),
            name: "Test Finding".to_string(),
            risk,
            url: "https://example.com/api".to_string(),
            method: "GET".to_string(),
            description: "A vulnerability".to_string(),
            solution: "Fix it".to_string(),
            cwe_id,
        }
    }

    #[test]
    fn test_risk_rank_ordering() {
        assert!(risk_rank(&RiskLevel::High) > risk_rank(&RiskLevel::Medium));
        assert!(risk_rank(&RiskLevel::Medium) > risk_rank(&RiskLevel::Low));
        assert!(risk_rank(&RiskLevel::Low) > risk_rank(&RiskLevel::Informational));
    }

    #[test]
    fn test_count_by_risk() {
        let findings = vec![
            make_finding(RiskLevel::High, None),
            make_finding(RiskLevel::High, None),
            make_finding(RiskLevel::Medium, None),
            make_finding(RiskLevel::Low, None),
            make_finding(RiskLevel::Informational, None),
        ];
        let counts = count_by_risk(&findings);
        assert_eq!(counts.get("High"), Some(&2));
        assert_eq!(counts.get("Medium"), Some(&1));
        assert_eq!(counts.get("Low"), Some(&1));
    }

    #[test]
    fn test_high_risk_findings_filter() {
        let findings = vec![
            make_finding(RiskLevel::High, None),
            make_finding(RiskLevel::Medium, None),
            make_finding(RiskLevel::High, None),
            make_finding(RiskLevel::Low, None),
        ];
        let high = high_risk_findings(&findings);
        assert_eq!(high.len(), 2);
        for f in &high {
            assert_eq!(f.risk, RiskLevel::High);
        }
    }

    #[test]
    fn test_scan_score_calculation() {
        let scan = DastScan {
            id: Uuid::new_v4(),
            target_url: "https://example.com".to_string(),
            scan_type: ScanType::Full,
            status: ScanStatus::Completed,
            created_at: Utc::now(),
            completed_at: None,
            findings: vec![
                make_finding(RiskLevel::High, None),   // 3
                make_finding(RiskLevel::Medium, None), // 2
                make_finding(RiskLevel::Low, None),    // 1
            ],
        };
        assert_eq!(scan_score(&scan), 6);
    }

    #[test]
    fn test_is_valid_url_https() {
        assert!(is_valid_url("https://example.com"));
        assert!(is_valid_url("http://localhost:8080"));
    }

    #[test]
    fn test_is_valid_url_invalid() {
        assert!(!is_valid_url("ftp://example.com"));
        assert!(!is_valid_url("example.com"));
        assert!(!is_valid_url(""));
    }

    #[test]
    fn test_findings_with_cwe() {
        let findings = vec![
            make_finding(RiskLevel::High, Some(89)),
            make_finding(RiskLevel::Medium, None),
            make_finding(RiskLevel::Low, Some(79)),
        ];
        let with_cwe = findings_with_cwe(&findings);
        assert_eq!(with_cwe.len(), 2);
        for f in &with_cwe {
            assert!(f.cwe_id.is_some());
        }
    }
}
