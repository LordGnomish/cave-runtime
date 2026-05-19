// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScanRule {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub pattern: String,
    pub rule_type: RuleType,
    pub severity: FindingSeverity,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RuleType {
    Regex,
    Keyword,
    Semgrep,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Critical,
    Major,
    Minor,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub id: Uuid,
    pub rule_id: Uuid,
    pub rule_name: String,
    pub file_path: String,
    pub line_number: usize,
    pub matched_text: String,
    pub severity: FindingSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub scan_id: Uuid,
    pub target: String,
    pub findings: Vec<Finding>,
    pub scanned_at: DateTime<Utc>,
    pub rules_applied: usize,
    pub files_scanned: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_rule(rule_type: RuleType, severity: FindingSeverity, enabled: bool) -> ScanRule {
        ScanRule {
            id: Uuid::new_v4(),
            name: "test-rule".to_string(),
            description: "A test rule".to_string(),
            pattern: "password".to_string(),
            rule_type,
            severity,
            enabled,
        }
    }

    fn make_finding(severity: FindingSeverity) -> Finding {
        Finding {
            id: Uuid::new_v4(),
            rule_id: Uuid::new_v4(),
            rule_name: "test-rule".to_string(),
            file_path: "src/main.rs".to_string(),
            line_number: 42,
            matched_text: "let password = \"secret\"".to_string(),
            severity,
            message: "Hardcoded password found".to_string(),
        }
    }

    #[test]
    fn test_rule_type_serde() {
        let variants = vec![RuleType::Regex, RuleType::Keyword, RuleType::Semgrep];
        for variant in variants {
            let json = serde_json::to_string(&variant).expect("serialize rule type");
            let back: RuleType = serde_json::from_str(&json).expect("deserialize rule type");
            assert_eq!(variant, back);
        }
    }

    #[test]
    fn test_finding_severity_serde() {
        let variants = vec![
            FindingSeverity::Critical,
            FindingSeverity::Major,
            FindingSeverity::Minor,
            FindingSeverity::Info,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).expect("serialize finding severity");
            let back: FindingSeverity = serde_json::from_str(&json).expect("deserialize finding severity");
            assert_eq!(variant, back);
        }
    }

    #[test]
    fn test_scan_rule_roundtrip() {
        let rule = make_rule(RuleType::Keyword, FindingSeverity::Major, true);
        let json = serde_json::to_string(&rule).expect("serialize scan rule");
        let back: ScanRule = serde_json::from_str(&json).expect("deserialize scan rule");
        assert_eq!(rule, back);
    }

    #[test]
    fn test_finding_roundtrip() {
        let finding = make_finding(FindingSeverity::Critical);
        let json = serde_json::to_string(&finding).expect("serialize finding");
        let back: Finding = serde_json::from_str(&json).expect("deserialize finding");
        assert_eq!(finding, back);
    }

    #[test]
    fn test_finding_severity_ordering() {
        use crate::engine::severity_rank;
        // Critical > Major > Minor > Info
        assert!(severity_rank(&FindingSeverity::Critical) > severity_rank(&FindingSeverity::Major));
        assert!(severity_rank(&FindingSeverity::Major) > severity_rank(&FindingSeverity::Minor));
        assert!(severity_rank(&FindingSeverity::Minor) > severity_rank(&FindingSeverity::Info));
    }
}