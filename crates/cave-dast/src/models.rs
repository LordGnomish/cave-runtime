// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DastScan {
    pub id: Uuid,
    pub target_url: String,
    pub scan_type: ScanType,
    pub status: ScanStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub findings: Vec<DastFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScanType {
    Baseline,
    Full,
    Api,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DastFinding {
    pub id: Uuid,
    pub name: String,
    pub risk: RiskLevel,
    pub url: String,
    pub method: String,
    pub description: String,
    pub solution: String,
    pub cwe_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    High,
    Medium,
    Low,
    Informational,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_finding(risk: RiskLevel, cwe_id: Option<u32>) -> DastFinding {
        DastFinding {
            id: Uuid::new_v4(),
            name: "SQL Injection".to_string(),
            risk,
            url: "https://example.com/api/users".to_string(),
            method: "GET".to_string(),
            description: "SQL injection vulnerability found".to_string(),
            solution: "Use parameterized queries".to_string(),
            cwe_id,
        }
    }

    #[test]
    fn test_risk_level_serialization() {
        let json = serde_json::to_string(&RiskLevel::High).unwrap();
        assert_eq!(json, "\"high\"");
        let back: RiskLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, RiskLevel::High);
    }

    #[test]
    fn test_scan_type_serialization() {
        let types = vec![ScanType::Baseline, ScanType::Full, ScanType::Api];
        for st in types {
            let json = serde_json::to_string(&st).unwrap();
            let back: ScanType = serde_json::from_str(&json).unwrap();
            assert_eq!(st, back);
        }
    }

    #[test]
    fn test_finding_roundtrip() {
        let finding = make_finding(RiskLevel::Medium, Some(89));
        let json = serde_json::to_string(&finding).unwrap();
        let back: DastFinding = serde_json::from_str(&json).unwrap();
        assert_eq!(finding, back);
    }

    #[test]
    fn test_scan_roundtrip() {
        let scan = DastScan {
            id: Uuid::new_v4(),
            target_url: "https://example.com".to_string(),
            scan_type: ScanType::Full,
            status: ScanStatus::Completed,
            created_at: Utc::now(),
            completed_at: Some(Utc::now()),
            findings: vec![make_finding(RiskLevel::High, Some(89))],
        };
        let json = serde_json::to_string(&scan).unwrap();
        let back: DastScan = serde_json::from_str(&json).unwrap();
        assert_eq!(scan, back);
    }

    #[test]
    fn test_finding_no_cwe() {
        let finding = make_finding(RiskLevel::Low, None);
        assert_eq!(finding.cwe_id, None);
        let json = serde_json::to_string(&finding).unwrap();
        let back: DastFinding = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cwe_id, None);
    }
}
