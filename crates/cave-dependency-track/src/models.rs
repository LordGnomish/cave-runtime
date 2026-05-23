// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core data types — mirrors `org.dependencytrack.model.*`.
//!
//! Source: `dependency-track/src/main/java/org/dependencytrack/model/`.
//! Pinned: v4.14.2 / `c4a156726472cd529cc9fa8ed12e825cc000327d`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Component classifier — mirrors `model/Classifier.java`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Classifier {
    Application,
    Framework,
    Library,
    Container,
    OperatingSystem,
    Device,
    Firmware,
    File,
}

impl Classifier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Classifier::Application => "APPLICATION",
            Classifier::Framework => "FRAMEWORK",
            Classifier::Library => "LIBRARY",
            Classifier::Container => "CONTAINER",
            Classifier::OperatingSystem => "OPERATING_SYSTEM",
            Classifier::Device => "DEVICE",
            Classifier::Firmware => "FIRMWARE",
            Classifier::File => "FILE",
        }
    }

    /// CycloneDX type tag — `library`, `application`, `container`, …
    pub fn cyclonedx_type(&self) -> &'static str {
        match self {
            Classifier::Application => "application",
            Classifier::Framework => "framework",
            Classifier::Library => "library",
            Classifier::Container => "container",
            Classifier::OperatingSystem => "operating-system",
            Classifier::Device => "device",
            Classifier::Firmware => "firmware",
            Classifier::File => "file",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "APPLICATION" => Some(Self::Application),
            "FRAMEWORK" => Some(Self::Framework),
            "LIBRARY" => Some(Self::Library),
            "CONTAINER" => Some(Self::Container),
            "OPERATING_SYSTEM" | "OPERATING-SYSTEM" | "OS" => Some(Self::OperatingSystem),
            "DEVICE" => Some(Self::Device),
            "FIRMWARE" => Some(Self::Firmware),
            "FILE" => Some(Self::File),
            _ => None,
        }
    }
}

/// Severity — mirrors `org.dependencytrack.model.Severity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Unassigned,
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// CVSS v3 base-score → severity bucket (Dependency-Track table).
    pub fn from_cvss_v3(score: f64) -> Self {
        match score {
            s if s == 0.0 => Severity::Info,
            s if s < 4.0 => Severity::Low,
            s if s < 7.0 => Severity::Medium,
            s if s < 9.0 => Severity::High,
            _ => Severity::Critical,
        }
    }

    pub fn rank(&self) -> u8 {
        match self {
            Severity::Unassigned => 0,
            Severity::Info => 1,
            Severity::Low => 2,
            Severity::Medium => 3,
            Severity::High => 4,
            Severity::Critical => 5,
        }
    }
}

/// Vulnerability analysis state — `model/AnalysisState.java`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum AnalysisState {
    NotSet,
    Exploitable,
    InTriage,
    Resolved,
    FalsePositive,
    NotAffected,
}

/// Justification for a NotAffected analysis — `AnalysisJustification.java`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisJustification {
    NotSet,
    CodeNotPresent,
    CodeNotReachable,
    RequiresConfiguration,
    RequiresDependency,
    RequiresEnvironment,
    ProtectedByCompiler,
    ProtectedAtRuntime,
    ProtectedAtPerimeter,
    ProtectedByMitigatingControl,
}

/// Response taken once a finding is triaged — `AnalysisResponse.java`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisResponse {
    NotSet,
    CanNotFix,
    WillNotFix,
    Update,
    RollBack,
    WorkaroundAvailable,
}

/// Vulnerability source — mirrors the `Vulnerability.Source` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum VulnSource {
    Nvd,
    Github,
    Osv,
    Ossindex,
    Snyk,
    Vulndb,
    Internal,
}

/// Project — minimal subset of `model/Project.java`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub uuid: Uuid,
    pub name: String,
    pub version: Option<String>,
    pub classifier: Classifier,
    pub group: Option<String>,
    pub purl: Option<String>,
    pub cpe: Option<String>,
    pub description: Option<String>,
    pub parent: Option<Uuid>,
    pub tags: Vec<String>,
    pub active: bool,
    pub created: DateTime<Utc>,
    pub last_bom_import: Option<DateTime<Utc>>,
    pub last_inherited_risk_score: Option<f64>,
    pub last_bom_import_format: Option<String>,
}

impl Project {
    pub fn new(name: impl Into<String>, classifier: Classifier) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            name: name.into(),
            version: None,
            classifier,
            group: None,
            purl: None,
            cpe: None,
            description: None,
            parent: None,
            tags: Vec::new(),
            active: true,
            created: Utc::now(),
            last_bom_import: None,
            last_inherited_risk_score: None,
            last_bom_import_format: None,
        }
    }
}

/// Component — minimal subset of `model/Component.java`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub uuid: Uuid,
    pub project: Uuid,
    pub name: String,
    pub version: Option<String>,
    pub group: Option<String>,
    pub purl: Option<String>,
    pub cpe: Option<String>,
    pub swid_tag_id: Option<String>,
    pub classifier: Classifier,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub sha512: Option<String>,
    pub license: Option<String>,
    pub license_expression: Option<String>,
    pub is_internal: bool,
}

impl Component {
    pub fn new(project: Uuid, name: impl Into<String>) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            project,
            name: name.into(),
            version: None,
            group: None,
            purl: None,
            cpe: None,
            swid_tag_id: None,
            classifier: Classifier::Library,
            md5: None,
            sha1: None,
            sha256: None,
            sha512: None,
            license: None,
            license_expression: None,
            is_internal: false,
        }
    }
}

/// Vulnerability — minimal subset of `model/Vulnerability.java`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vulnerability {
    pub uuid: Uuid,
    pub vuln_id: String,
    pub source: VulnSource,
    pub title: Option<String>,
    pub description: Option<String>,
    pub severity: Severity,
    pub cvss_v3_base_score: Option<f64>,
    pub cvss_v3_vector: Option<String>,
    pub epss_score: Option<f64>,
    pub epss_percentile: Option<f64>,
    pub cwes: Vec<u32>,
    pub published: Option<DateTime<Utc>>,
    pub updated: Option<DateTime<Utc>>,
}

impl Vulnerability {
    pub fn new(vuln_id: impl Into<String>, source: VulnSource) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            vuln_id: vuln_id.into(),
            source,
            title: None,
            description: None,
            severity: Severity::Unassigned,
            cvss_v3_base_score: None,
            cvss_v3_vector: None,
            epss_score: None,
            epss_percentile: None,
            cwes: Vec::new(),
            published: None,
            updated: None,
        }
    }
}

/// Tag — many-to-many label on a Project.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_roundtrip() {
        for c in [
            Classifier::Application,
            Classifier::Framework,
            Classifier::Library,
            Classifier::Container,
            Classifier::OperatingSystem,
            Classifier::Device,
            Classifier::Firmware,
            Classifier::File,
        ] {
            assert_eq!(Classifier::parse(c.as_str()), Some(c));
        }
        assert_eq!(Classifier::parse("OPERATING-SYSTEM"), Some(Classifier::OperatingSystem));
        assert_eq!(Classifier::parse("nope"), None);
    }

    #[test]
    fn classifier_cyclonedx_lowercases() {
        assert_eq!(Classifier::Library.cyclonedx_type(), "library");
        assert_eq!(Classifier::OperatingSystem.cyclonedx_type(), "operating-system");
    }

    #[test]
    fn severity_from_cvss_v3_table_buckets() {
        assert_eq!(Severity::from_cvss_v3(0.0), Severity::Info);
        assert_eq!(Severity::from_cvss_v3(3.9), Severity::Low);
        assert_eq!(Severity::from_cvss_v3(4.0), Severity::Medium);
        assert_eq!(Severity::from_cvss_v3(6.9), Severity::Medium);
        assert_eq!(Severity::from_cvss_v3(7.0), Severity::High);
        assert_eq!(Severity::from_cvss_v3(8.9), Severity::High);
        assert_eq!(Severity::from_cvss_v3(9.0), Severity::Critical);
        assert_eq!(Severity::from_cvss_v3(10.0), Severity::Critical);
    }

    #[test]
    fn severity_rank_orders_ascending() {
        assert!(Severity::Low.rank() < Severity::Medium.rank());
        assert!(Severity::High.rank() < Severity::Critical.rank());
        assert!(Severity::Unassigned < Severity::Critical);
    }

    #[test]
    fn project_new_defaults_active_and_no_parent() {
        let p = Project::new("cave", Classifier::Application);
        assert!(p.active);
        assert!(p.parent.is_none());
        assert!(p.tags.is_empty());
        assert!(p.last_bom_import.is_none());
    }

    #[test]
    fn component_new_defaults_library() {
        let pid = Uuid::nil();
        let c = Component::new(pid, "lib");
        assert_eq!(c.classifier, Classifier::Library);
        assert_eq!(c.project, pid);
        assert!(!c.is_internal);
    }

    #[test]
    fn vulnerability_new_unassigned_severity() {
        let v = Vulnerability::new("CVE-2026-0001", VulnSource::Nvd);
        assert_eq!(v.severity, Severity::Unassigned);
        assert_eq!(v.source, VulnSource::Nvd);
    }

    #[test]
    fn analysis_state_serializes_uppercase() {
        let s = serde_json::to_string(&AnalysisState::Exploitable).unwrap();
        assert_eq!(s, "\"EXPLOITABLE\"");
    }

    #[test]
    fn analysis_response_serializes_snake_case() {
        let s = serde_json::to_string(&AnalysisResponse::WillNotFix).unwrap();
        assert_eq!(s, "\"will_not_fix\"");
    }
}
