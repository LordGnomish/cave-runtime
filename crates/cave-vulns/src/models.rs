// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Vulnerability {
    pub id: Uuid,
    pub cve_id: String,
    pub title: String,
    pub description: String,
    pub severity: Severity,
    pub cvss_score: f32,
    pub affected_component: String,
    pub affected_versions: Vec<String>,
    pub fixed_in: Option<String>,
    pub published_at: DateTime<Utc>,
    pub state: VulnState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VulnState {
    Open,
    Acknowledged,
    Mitigated,
    FalsePositive,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VulnScanResult {
    pub scan_id: Uuid,
    pub target: String,
    pub findings: Vec<Vulnerability>,
    pub scanned_at: DateTime<Utc>,
    pub total_critical: usize,
    pub total_high: usize,
    pub total_medium: usize,
    pub total_low: usize,
}

#[derive(Debug, Deserialize)]
pub struct ScanRequest {
    pub target: String,
    pub components: Vec<ComponentVersion>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComponentVersion {
    pub name: String,
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_vuln(severity: Severity) -> Vulnerability {
        Vulnerability {
            id: Uuid::new_v4(),
            cve_id: "CVE-2024-1234".to_string(),
            title: "Test Vulnerability".to_string(),
            description: "A test vulnerability".to_string(),
            severity,
            cvss_score: 7.5,
            affected_component: "openssl".to_string(),
            affected_versions: vec!["1.0.1".to_string(), "1.0.2".to_string()],
            fixed_in: Some("1.0.3".to_string()),
            published_at: Utc::now(),
            state: VulnState::Open,
        }
    }

    #[test]
    fn test_severity_serde_roundtrip() {
        let variants = vec![
            Severity::Critical,
            Severity::High,
            Severity::Medium,
            Severity::Low,
            Severity::Info,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).expect("serialize severity");
            let back: Severity = serde_json::from_str(&json).expect("deserialize severity");
            assert_eq!(variant, back);
        }
    }

    #[test]
    fn test_vuln_state_serde_roundtrip() {
        let variants = vec![
            VulnState::Open,
            VulnState::Acknowledged,
            VulnState::Mitigated,
            VulnState::FalsePositive,
            VulnState::Resolved,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).expect("serialize vuln state");
            let back: VulnState = serde_json::from_str(&json).expect("deserialize vuln state");
            assert_eq!(variant, back);
        }
    }

    #[test]
    fn test_vulnerability_serde_roundtrip() {
        let vuln = make_vuln(Severity::High);
        let json = serde_json::to_string(&vuln).expect("serialize vulnerability");
        let back: Vulnerability = serde_json::from_str(&json).expect("deserialize vulnerability");
        assert_eq!(vuln, back);
    }

    #[test]
    fn test_vuln_scan_result_serializes() {
        let result = VulnScanResult {
            scan_id: Uuid::new_v4(),
            target: "my-service".to_string(),
            findings: vec![make_vuln(Severity::Critical)],
            scanned_at: Utc::now(),
            total_critical: 1,
            total_high: 0,
            total_medium: 0,
            total_low: 0,
        };
        let json = serde_json::to_string(&result).expect("serialize scan result");
        assert!(json.contains("scan_id"));
        assert!(json.contains("my-service"));
    }

    #[test]
    fn test_component_version_deserialize() {
        let json = r#"{"name":"openssl","version":"1.0.1"}"#;
        let cv: ComponentVersion = serde_json::from_str(json).expect("deserialize component version");
        assert_eq!(cv.name, "openssl");
        assert_eq!(cv.version, "1.0.1");
    }
}
