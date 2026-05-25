// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Core scan types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ScanKind {
    Image,
    Iac,
    Fs,
    Secret,
    Yara,
    Namespace,
}

impl std::fmt::Display for ScanKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ScanKind::Image => "image",
            ScanKind::Iac => "iac",
            ScanKind::Fs => "fs",
            ScanKind::Secret => "secret",
            ScanKind::Yara => "yara",
            ScanKind::Namespace => "namespace",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ScanTarget {
    #[serde(rename = "image_ref")]
    ImageRef(String),
    #[serde(rename = "fs_path")]
    FsPath(String),
    #[serde(rename = "content")]
    Content(Vec<u8>),
    #[serde(rename = "package_name")]
    PackageName { ecosystem: Ecosystem, name: String },
    #[serde(rename = "iac_bundle")]
    IacBundle { kind: IacKind, content: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IacKind {
    Dockerfile,
    Kubernetes,
    Terraform,
    HelmChart,
    DockerCompose,
}

impl std::fmt::Display for IacKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            IacKind::Dockerfile => "dockerfile",
            IacKind::Kubernetes => "kubernetes",
            IacKind::Terraform => "terraform",
            IacKind::HelmChart => "helm_chart",
            IacKind::DockerCompose => "docker_compose",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Ecosystem {
    PyPI,
    Npm,
    Maven,
    RubyGems,
    Cargo,
    Go,
    NuGet,
    Composer,
    Oci,
}

impl std::fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Ecosystem::PyPI => "PyPI",
            Ecosystem::Npm => "Npm",
            Ecosystem::Maven => "Maven",
            Ecosystem::RubyGems => "RubyGems",
            Ecosystem::Cargo => "Cargo",
            Ecosystem::Go => "Go",
            Ecosystem::NuGet => "NuGet",
            Ecosystem::Composer => "Composer",
            Ecosystem::Oci => "OCI",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOptions {
    pub timeout_seconds: Option<u32>,
    pub skip_rules: Vec<String>,
    pub include_only: Option<Vec<String>>,
    pub severity_floor: Option<Severity>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            timeout_seconds: None,
            skip_rules: vec![],
            include_only: None,
            severity_floor: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanRequest {
    pub kind: ScanKind,
    pub target: ScanTarget,
    pub options: ScanOptions,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    // Ordered lowest → highest so derived Ord gives Info < Low < … < Critical.
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn as_str(&self) -> &str {
        match self {
            Severity::Critical => "critical",
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
            Severity::Info => "info",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Confirmed,
    High,
    Medium,
    Low,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Confidence::Confirmed => "confirmed",
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    KnownVulnerability,
    Misconfig,
    ExposedSecret,
    Malware,
    Typosquat,
    LicenseViolation,
    InsecureDefaults,
    SupplyChainAnomaly,
}

impl std::fmt::Display for FindingCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            FindingCategory::KnownVulnerability => "known_vulnerability",
            FindingCategory::Misconfig => "misconfig",
            FindingCategory::ExposedSecret => "exposed_secret",
            FindingCategory::Malware => "malware",
            FindingCategory::Typosquat => "typosquat",
            FindingCategory::LicenseViolation => "license_violation",
            FindingCategory::InsecureDefaults => "insecure_defaults",
            FindingCategory::SupplyChainAnomaly => "supply_chain_anomaly",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingLocation {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub layer_digest: Option<String>,
    pub package: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: Uuid,
    pub rule_id: String,
    pub rule_name: String,
    pub category: FindingCategory,
    pub severity: Severity,
    pub title: String,
    pub description: String,
    pub location: FindingLocation,
    pub cves: Vec<String>,
    pub cwes: Vec<String>,
    pub remediation: Option<String>,
    pub confidence: Confidence,
    pub evidence: Option<String>,
    pub fingerprint: String,
}

impl Finding {
    pub fn new(
        rule_id: String,
        rule_name: String,
        category: FindingCategory,
        severity: Severity,
        title: String,
        description: String,
    ) -> Self {
        let fingerprint = format!("{}:{}:{}", rule_id, title, severity.as_str());
        Self {
            id: Uuid::new_v4(),
            rule_id,
            rule_name,
            category,
            severity,
            title,
            description,
            location: FindingLocation {
                file: None,
                line: None,
                column: None,
                layer_digest: None,
                package: None,
                version: None,
            },
            cves: vec![],
            cwes: vec![],
            remediation: None,
            confidence: Confidence::Medium,
            evidence: None,
            fingerprint,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Pending,
    Running,
    Completed,
    Failed,
    TimedOut,
}

impl std::fmt::Display for ScanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ScanStatus::Pending => "pending",
            ScanStatus::Running => "running",
            ScanStatus::Completed => "completed",
            ScanStatus::Failed => "failed",
            ScanStatus::TimedOut => "timed_out",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub id: Uuid,
    pub request: ScanRequest,
    pub findings: Vec<Finding>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub scanner_version: String,
    pub status: ScanStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerdictDecision {
    Pass,
    Warn,
    Fail,
}

impl std::fmt::Display for VerdictDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            VerdictDecision::Pass => "pass",
            VerdictDecision::Warn => "warn",
            VerdictDecision::Fail => "fail",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanVerdict {
    pub decision: VerdictDecision,
    pub reasons: Vec<String>,
    pub finding_ids: Vec<Uuid>,
    pub evaluated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageLayer {
    pub digest: String,
    pub size_bytes: u64,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    pub variant: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageManifest {
    pub digest: String,
    pub layers: Vec<ImageLayer>,
    pub platform: Platform,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub ecosystem: Ecosystem,
    pub licenses: Vec<String>,
    pub source_layer: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanStats {
    pub total_scans: u64,
    pub pass: u64,
    pub warn: u64,
    pub fail: u64,
    pub findings_by_severity: HashMap<Severity, u64>,
}

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ImageScanRequest {
    pub r#ref: String,
    pub platform: Option<Platform>,
}

#[derive(Debug, Deserialize)]
pub struct IacScanRequest {
    pub kind: IacKind,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct FsScanRequest {
    pub path: Option<String>,
    pub entries: Option<Vec<FsEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FsEntry {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct SecretScanRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct YaraScanRequest {
    pub bytes: Vec<u8>,
}

#[derive(Debug, Deserialize)]
pub struct NamespaceScanRequest {
    pub ecosystem: Ecosystem,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct VerdictRequest {
    pub findings: Vec<Finding>,
    pub floor: Option<Severity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RulesResponse {
    pub yara: Vec<YaraRuleMetadata>,
    pub iac: Vec<IacRuleMetadata>,
    pub secret: Vec<SecretRuleMetadata>,
    pub namespace: Vec<NamespaceRuleMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct YaraRuleMetadata {
    pub id: String,
    pub name: String,
    pub patterns: Vec<String>,
    pub severity: Severity,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IacRuleMetadata {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub severity: Severity,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SecretRuleMetadata {
    pub id: String,
    pub name: String,
    pub pattern: String,
    pub severity: Severity,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NamespaceRuleMetadata {
    pub id: String,
    pub name: String,
    pub ecosystem: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_kind_display() {
        assert_eq!(ScanKind::Image.to_string(), "image");
        assert_eq!(ScanKind::Iac.to_string(), "iac");
        assert_eq!(ScanKind::Fs.to_string(), "fs");
        assert_eq!(ScanKind::Secret.to_string(), "secret");
        assert_eq!(ScanKind::Yara.to_string(), "yara");
        assert_eq!(ScanKind::Namespace.to_string(), "namespace");
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Info);
    }

    #[test]
    fn test_finding_new() {
        let f = Finding::new(
            "TEST-001".to_string(),
            "Test Rule".to_string(),
            FindingCategory::Misconfig,
            Severity::High,
            "Test finding".to_string(),
            "This is a test".to_string(),
        );
        assert_eq!(f.rule_id, "TEST-001");
        assert!(!f.fingerprint.is_empty());
    }

    #[test]
    fn test_scan_stats_default() {
        let s = ScanStats::default();
        assert_eq!(s.total_scans, 0);
        assert_eq!(s.pass, 0);
    }

    #[test]
    fn test_ecosystem_serialization() {
        let e = Ecosystem::PyPI;
        let json = serde_json::to_string(&e).unwrap();
        let decoded: Ecosystem = serde_json::from_str(&json).unwrap();
        assert_eq!(e, decoded);
    }

    #[test]
    fn test_iac_kind_display() {
        assert_eq!(IacKind::Dockerfile.to_string(), "dockerfile");
        assert_eq!(IacKind::Kubernetes.to_string(), "kubernetes");
        assert_eq!(IacKind::Terraform.to_string(), "terraform");
    }

    #[test]
    fn test_verdict_decision_serialization() {
        let v = VerdictDecision::Fail;
        let json = serde_json::to_string(&v).unwrap();
        let decoded: VerdictDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(v, decoded);
    }
}
