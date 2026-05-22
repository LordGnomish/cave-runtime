// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/model/{Component,Vulnerability,License,Cwe,Classifier}.java
//
//! Core data shapes mirroring Dependency-Track JDO models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sbom {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub format: SbomFormat,
    pub components: Vec<Component>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SbomFormat {
    CycloneDx,
    Spdx,
    Syft,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub version: String,
    pub purl: Option<String>,
    pub license: Option<String>,
    pub component_type: ComponentType,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ComponentType {
    Library,
    Application,
    Container,
    Device,
    Firmware,
    File,
    OperatingSystem,
    Framework,
}

impl ComponentType {
    /// Mirror of `org.dependencytrack.model.Classifier` enum string.
    pub fn as_classifier(&self) -> &'static str {
        match self {
            ComponentType::Library => "LIBRARY",
            ComponentType::Application => "APPLICATION",
            ComponentType::Container => "CONTAINER",
            ComponentType::Device => "DEVICE",
            ComponentType::Firmware => "FIRMWARE",
            ComponentType::File => "FILE",
            ComponentType::OperatingSystem => "OPERATING_SYSTEM",
            ComponentType::Framework => "FRAMEWORK",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DependencyTree {
    pub root: String,
    pub adjacency: HashMap<String, Vec<String>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Vulnerability intelligence shapes.
// Mirrors org.dependencytrack.model.Vulnerability.
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregated vulnerability advisory shape (NVD / OSV / GHSA / Snyk merge target).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VulnIntel {
    pub id: Uuid,
    /// Primary identifier (e.g. CVE-2024-12345, GHSA-xxxx, OSV-2024-xxx).
    pub vuln_id: String,
    pub source: VulnSource,
    pub title: String,
    pub description: String,
    pub severity: Severity,
    pub cvss_v3_base: Option<f32>,
    pub cvss_v3_vector: Option<String>,
    pub cvss_v2_base: Option<f32>,
    /// Optional EPSS score in [0.0, 1.0]. Joined from `vuln_intel::epss`.
    pub epss_score: Option<f32>,
    /// Optional EPSS percentile in [0.0, 1.0].
    pub epss_percentile: Option<f32>,
    pub cwes: Vec<u32>,
    pub references: Vec<String>,
    pub affected: Vec<AffectedRange>,
    pub published: Option<DateTime<Utc>>,
    pub modified: Option<DateTime<Utc>>,
    pub state: AnalysisState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "UPPERCASE")]
pub enum VulnSource {
    Nvd,
    Osv,
    Ghsa,
    Snyk,
    VulnDb,
    Github,
    Internal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    /// Sort order matters: Unassigned < Info < Low < Medium < High < Critical.
    Unassigned,
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// Convert CVSS v3 base score to severity bucket (NVD methodology).
    /// Source: NVD CVSS v3 severity rating table.
    pub fn from_cvss_v3(score: f32) -> Self {
        match score {
            s if s >= 9.0 => Severity::Critical,
            s if s >= 7.0 => Severity::High,
            s if s >= 4.0 => Severity::Medium,
            s if s > 0.0 => Severity::Low,
            _ => Severity::Info,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalysisState {
    /// Mirror of `org.dependencytrack.model.AnalysisState`.
    NotSet,
    Exploitable,
    InTriage,
    Resolved,
    FalsePositive,
    NotAffected,
}

/// Range of versions impacted by a vulnerability — VERS semantics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AffectedRange {
    /// PURL or package coordinates (purl pkg:npm/lodash, etc.).
    pub purl_type: String,
    pub namespace: Option<String>,
    pub name: String,
    /// VERS range expression (e.g. ">=4.0.0 <4.17.21").
    pub vers: String,
    /// Fixed-in version, when known.
    pub fixed: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_component(id: &str, ct: ComponentType) -> Component {
        Component {
            id: id.to_string(),
            name: id.to_string(),
            version: "1.0.0".to_string(),
            purl: None,
            license: None,
            component_type: ct,
            dependencies: vec![],
        }
    }

    #[test]
    fn test_sbom_format_serde() {
        assert_eq!(
            serde_json::to_string(&SbomFormat::CycloneDx).unwrap(),
            "\"cyclone_dx\""
        );
        assert_eq!(
            serde_json::to_string(&SbomFormat::Spdx).unwrap(),
            "\"spdx\""
        );
        assert_eq!(
            serde_json::to_string(&SbomFormat::Syft).unwrap(),
            "\"syft\""
        );
    }

    #[test]
    fn test_component_type_serde() {
        assert_eq!(
            serde_json::to_string(&ComponentType::Library).unwrap(),
            "\"library\""
        );
        assert_eq!(
            serde_json::to_string(&ComponentType::Application).unwrap(),
            "\"application\""
        );
    }

    #[test]
    fn test_component_type_classifier_strings() {
        assert_eq!(ComponentType::Library.as_classifier(), "LIBRARY");
        assert_eq!(
            ComponentType::OperatingSystem.as_classifier(),
            "OPERATING_SYSTEM"
        );
        assert_eq!(ComponentType::Framework.as_classifier(), "FRAMEWORK");
    }

    #[test]
    fn test_component_serde_roundtrip() {
        let comp = Component {
            id: "c1".to_string(),
            name: "lodash".to_string(),
            version: "4.17.21".to_string(),
            purl: Some("pkg:npm/lodash@4.17.21".to_string()),
            license: Some("MIT".to_string()),
            component_type: ComponentType::Library,
            dependencies: vec!["c2".to_string()],
        };
        let json = serde_json::to_string(&comp).unwrap();
        let back: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(comp, back);
    }

    #[test]
    fn test_sbom_serde_roundtrip() {
        let sbom = Sbom {
            id: Uuid::new_v4(),
            name: "my-app".to_string(),
            version: "1.0.0".to_string(),
            format: SbomFormat::CycloneDx,
            components: vec![make_component("c1", ComponentType::Library)],
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&sbom).unwrap();
        let back: Sbom = serde_json::from_str(&json).unwrap();
        assert_eq!(sbom, back);
    }

    #[test]
    fn test_component_no_purl_or_license() {
        let comp = make_component("bare", ComponentType::Container);
        let json = serde_json::to_string(&comp).unwrap();
        let back: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(comp, back);
        assert!(back.purl.is_none());
        assert!(back.license.is_none());
    }

    #[test]
    fn test_component_with_multiple_deps() {
        let mut comp = make_component("root", ComponentType::Application);
        comp.dependencies = vec!["dep1".to_string(), "dep2".to_string(), "dep3".to_string()];
        let json = serde_json::to_string(&comp).unwrap();
        let back: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dependencies.len(), 3);
    }

    #[test]
    fn test_sbom_format_deserialization() {
        let f: SbomFormat = serde_json::from_str("\"syft\"").unwrap();
        assert_eq!(f, SbomFormat::Syft);
    }

    #[test]
    fn test_component_type_all_variants() {
        for ct in [
            ComponentType::Library,
            ComponentType::Application,
            ComponentType::Container,
            ComponentType::Device,
            ComponentType::Firmware,
            ComponentType::File,
            ComponentType::OperatingSystem,
            ComponentType::Framework,
        ] {
            let json = serde_json::to_string(&ct).unwrap();
            let back: ComponentType = serde_json::from_str(&json).unwrap();
            assert_eq!(ct, back);
        }
    }

    #[test]
    fn severity_ordering_matches_nvd_buckets() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Info);
        assert!(Severity::Info > Severity::Unassigned);
    }

    #[test]
    fn severity_from_cvss_v3_buckets() {
        assert_eq!(Severity::from_cvss_v3(9.8), Severity::Critical);
        assert_eq!(Severity::from_cvss_v3(9.0), Severity::Critical);
        assert_eq!(Severity::from_cvss_v3(8.9), Severity::High);
        assert_eq!(Severity::from_cvss_v3(7.0), Severity::High);
        assert_eq!(Severity::from_cvss_v3(6.9), Severity::Medium);
        assert_eq!(Severity::from_cvss_v3(4.0), Severity::Medium);
        assert_eq!(Severity::from_cvss_v3(3.9), Severity::Low);
        assert_eq!(Severity::from_cvss_v3(0.1), Severity::Low);
        assert_eq!(Severity::from_cvss_v3(0.0), Severity::Info);
    }

    #[test]
    fn vuln_source_serde_uppercase() {
        assert_eq!(serde_json::to_string(&VulnSource::Nvd).unwrap(), "\"NVD\"");
        assert_eq!(
            serde_json::to_string(&VulnSource::Ghsa).unwrap(),
            "\"GHSA\""
        );
        assert_eq!(serde_json::to_string(&VulnSource::Osv).unwrap(), "\"OSV\"");
    }

    #[test]
    fn vuln_intel_roundtrip() {
        let v = VulnIntel {
            id: Uuid::new_v4(),
            vuln_id: "CVE-2024-12345".into(),
            source: VulnSource::Nvd,
            title: "test".into(),
            description: "desc".into(),
            severity: Severity::High,
            cvss_v3_base: Some(7.5),
            cvss_v3_vector: Some("AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N".into()),
            cvss_v2_base: None,
            epss_score: Some(0.42),
            epss_percentile: Some(0.97),
            cwes: vec![79],
            references: vec!["https://example.com".into()],
            affected: vec![AffectedRange {
                purl_type: "npm".into(),
                namespace: None,
                name: "lodash".into(),
                vers: ">=4.0.0 <4.17.21".into(),
                fixed: Some("4.17.21".into()),
            }],
            published: Some(Utc::now()),
            modified: Some(Utc::now()),
            state: AnalysisState::NotSet,
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: VulnIntel = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn analysis_state_serde_uppercase() {
        assert_eq!(
            serde_json::to_string(&AnalysisState::NotSet).unwrap(),
            "\"NOT_SET\""
        );
        assert_eq!(
            serde_json::to_string(&AnalysisState::FalsePositive).unwrap(),
            "\"FALSE_POSITIVE\""
        );
        assert_eq!(
            serde_json::to_string(&AnalysisState::InTriage).unwrap(),
            "\"IN_TRIAGE\""
        );
    }
}
