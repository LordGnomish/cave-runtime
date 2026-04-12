//! Data models for cave-security.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Priority levels for security rules (matches Falco priority levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Priority {
    Debug = 0,
    Informational = 1,
    Notice = 2,
    Warning = 3,
    Error = 4,
    Critical = 5,
    Alert = 6,
    Emergency = 7,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::Debug => write!(f, "DEBUG"),
            Priority::Informational => write!(f, "INFO"),
            Priority::Notice => write!(f, "NOTICE"),
            Priority::Warning => write!(f, "WARNING"),
            Priority::Error => write!(f, "ERROR"),
            Priority::Critical => write!(f, "CRITICAL"),
            Priority::Alert => write!(f, "ALERT"),
            Priority::Emergency => write!(f, "EMERGENCY"),
        }
    }
}

/// Condition types for matching security events (Falco-style rule conditions).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    ProcessName { value: String, exact: bool },
    FilePath { prefix: String },
    NetworkPort { port: u16 },
    IsRoot,
    Syscall { name: String },
    ContainerImage { prefix: String },
    And { conditions: Vec<Condition> },
    Or { conditions: Vec<Condition> },
    Not { condition: Box<Condition> },
}

/// A security rule (Falco-style).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRule {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub priority: Priority,
    pub condition: Condition,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SecurityRule {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        priority: Priority,
        condition: Condition,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            priority,
            condition,
            tags: Vec::new(),
            enabled: true,
            created_at: now,
            updated_at: now,
        }
    }
}

/// A security event that can be evaluated against rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityEvent {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub process_name: Option<String>,
    pub file_path: Option<String>,
    pub network_port: Option<u16>,
    pub is_root: bool,
    pub syscall: Option<String>,
    pub container_image: Option<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    ProcessExec,
    FileAccess,
    NetworkConnect,
    PrivilegeEscalation,
    SyscallDetected,
}

/// An alert generated when a rule matches an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityAlert {
    pub id: Uuid,
    pub rule_id: Uuid,
    pub rule_name: String,
    pub priority: Priority,
    pub message: String,
    pub event: SecurityEvent,
    pub timestamp: DateTime<Utc>,
    pub acknowledged: bool,
}

/// CVSS severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CvssSeverity {
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

impl std::fmt::Display for CvssSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CvssSeverity::None => write!(f, "NONE"),
            CvssSeverity::Low => write!(f, "LOW"),
            CvssSeverity::Medium => write!(f, "MEDIUM"),
            CvssSeverity::High => write!(f, "HIGH"),
            CvssSeverity::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// A CVE entry in the vulnerability database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CveEntry {
    pub cve_id: String,
    pub description: String,
    pub severity: CvssSeverity,
    pub cvss_score: f32,
    pub affected_package: String,
    pub affected_versions: Vec<String>,
    pub fixed_version: Option<String>,
    pub published_at: DateTime<Utc>,
    pub references: Vec<String>,
}

/// A vulnerability finding from an image scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vulnerability {
    pub id: Uuid,
    pub cve_id: String,
    pub package_name: String,
    pub installed_version: String,
    pub fixed_version: Option<String>,
    pub severity: CvssSeverity,
    pub cvss_score: f32,
    pub description: String,
    pub layer_digest: Option<String>,
}

/// Scan policy evaluation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyResult {
    Pass,
    Fail { reasons: Vec<String> },
}

/// A container image scan result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub id: Uuid,
    pub image_reference: String,
    pub image_digest: String,
    pub scanned_at: DateTime<Utc>,
    pub vulnerabilities: Vec<Vulnerability>,
    pub policy_result: PolicyResult,
    pub signature_verified: bool,
}

/// Scan policy: what severity to fail on, CVE allowlist, signature requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanPolicy {
    pub id: Uuid,
    pub name: String,
    pub fail_on_severity: CvssSeverity,
    pub allowed_cves: Vec<String>,
    pub require_signature: bool,
    pub enabled: bool,
}

impl Default for ScanPolicy {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "default".to_string(),
            fail_on_severity: CvssSeverity::High,
            allowed_cves: Vec::new(),
            require_signature: false,
            enabled: true,
        }
    }
}

/// A component entry in an SBOM document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbomComponent {
    pub name: String,
    pub version: String,
    pub purl: String,
    pub licenses: Vec<String>,
    pub supplier: Option<String>,
    pub checksum_sha256: Option<String>,
}

/// SBOM output format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SbomFormat {
    Spdx,
    CycloneDx,
}

/// A Software Bill of Materials document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbomDocument {
    pub id: Uuid,
    pub format: SbomFormat,
    pub created_at: DateTime<Utc>,
    pub image_reference: String,
    pub components: Vec<SbomComponent>,
}
