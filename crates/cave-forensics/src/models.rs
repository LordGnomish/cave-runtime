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
