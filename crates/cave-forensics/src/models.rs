// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForensicCase {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub severity: ForensicSeverity,
    pub status: CaseStatus,
    pub created_at: DateTime<Utc>,
    pub evidence: Vec<EvidenceItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ForensicSeverity {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Open,
    InProgress,
    Closed,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceItem {
    pub id: Uuid,
    pub evidence_type: EvidenceType,
    pub description: String,
    pub hash_sha256: Option<String>,
    pub collected_at: DateTime<Utc>,
    pub chain_of_custody: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    NetworkCapture,
    ProcessDump,
    FileSystem,
    MemoryImage,
    LogFile,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn evidence(t: EvidenceType) -> EvidenceItem {
        EvidenceItem {
            id: Uuid::new_v4(),
            evidence_type: t,
            description: "e".to_string(),
            hash_sha256: Some("a".repeat(64)),
            collected_at: Utc::now(),
            chain_of_custody: vec!["alice".to_string(), "bob".to_string()],
        }
    }

    #[test]
    fn test_evidence_item_serde_roundtrip() {
        let ev = evidence(EvidenceType::NetworkCapture);
        let json = serde_json::to_string(&ev).unwrap();
        let back: EvidenceItem = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn test_forensic_case_serde_roundtrip() {
        let case = ForensicCase {
            id: Uuid::new_v4(),
            title: "incident-1".to_string(),
            description: "container escape attempt".to_string(),
            severity: ForensicSeverity::Critical,
            status: CaseStatus::InProgress,
            created_at: Utc::now(),
            evidence: vec![evidence(EvidenceType::ProcessDump)],
        };
        let json = serde_json::to_string(&case).unwrap();
        let back: ForensicCase = serde_json::from_str(&json).unwrap();
        assert_eq!(case, back);
    }

    #[test]
    fn test_evidence_type_all_variants_serde() {
        let variants = [
            (EvidenceType::NetworkCapture, "network_capture"),
            (EvidenceType::ProcessDump,    "process_dump"),
            (EvidenceType::FileSystem,     "file_system"),
            (EvidenceType::MemoryImage,    "memory_image"),
            (EvidenceType::LogFile,        "log_file"),
        ];
        for (v, expected_str) in variants {
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(json, format!("\"{}\"", expected_str));
            let back: EvidenceType = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn test_case_status_transitions_serde() {
        for status in [
            CaseStatus::Open,
            CaseStatus::InProgress,
            CaseStatus::Closed,
            CaseStatus::Archived,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: CaseStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }

    #[test]
    fn test_forensic_severity_serde_snake_case() {
        let json = serde_json::to_string(&ForensicSeverity::Critical).unwrap();
        assert_eq!(json, "\"critical\"");
        let back: ForensicSeverity = serde_json::from_str("\"high\"").unwrap();
        assert_eq!(back, ForensicSeverity::High);
    }

    #[test]
    fn test_chain_of_custody_preserved() {
        let ev = evidence(EvidenceType::LogFile);
        let json = serde_json::to_string(&ev).unwrap();
        let back: EvidenceItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.chain_of_custody, vec!["alice", "bob"]);
    }

    #[test]
    fn test_evidence_optional_hash_none() {
        let mut ev = evidence(EvidenceType::FileSystem);
        ev.hash_sha256 = None;
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"hash_sha256\":null"));
    }
}
