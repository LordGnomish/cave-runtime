// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Core data model for cave-trivy.
//!
//! Mirrors trivy's `pkg/types` plus `pkg/fanal/types` distilled to the
//! subset cave-trivy emits and consumes: `Severity`, `ScanTarget`,
//! `Package`, `Vulnerability`, `Misconfiguration`, `Secret`, `License`,
//! `ScanResult`, `Report`.

use crate::severity::Severity;
use serde::{Deserialize, Serialize};

/// Surface every cave-trivy scanner can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanTarget {
    Image,
    Filesystem,
    Repo,
    K8s,
    Sbom,
    Secret,
    Config,
}

impl ScanTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScanTarget::Image => "image",
            ScanTarget::Filesystem => "fs",
            ScanTarget::Repo => "repo",
            ScanTarget::K8s => "k8s",
            ScanTarget::Sbom => "sbom",
            ScanTarget::Secret => "secret",
            ScanTarget::Config => "config",
        }
    }
}

/// Operating-system family for OS-level package detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OsFamily {
    Alpine,
    Debian,
    Ubuntu,
    Rhel,
    Centos,
    Rocky,
    Alma,
    Amazon,
    Oracle,
    Photon,
    Mariner,
    Suse,
    OpenSuse,
    Unknown,
}

impl OsFamily {
    pub fn from_id(id: &str) -> Self {
        match id.to_ascii_lowercase().as_str() {
            "alpine" => OsFamily::Alpine,
            "debian" => OsFamily::Debian,
            "ubuntu" => OsFamily::Ubuntu,
            "rhel" | "redhat" => OsFamily::Rhel,
            "centos" => OsFamily::Centos,
            "rocky" => OsFamily::Rocky,
            "alma" | "almalinux" => OsFamily::Alma,
            "amzn" | "amazon" => OsFamily::Amazon,
            "ol" | "oracle" => OsFamily::Oracle,
            "photon" => OsFamily::Photon,
            "mariner" => OsFamily::Mariner,
            "sles" | "suse" => OsFamily::Suse,
            "opensuse" => OsFamily::OpenSuse,
            _ => OsFamily::Unknown,
        }
    }
}

/// One installed package (OS or language).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub ecosystem: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purl: Option<String>,
}

impl Package {
    pub fn new(name: &str, version: &str, ecosystem: &str) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            ecosystem: ecosystem.into(),
            source: None,
            release: None,
            layer_digest: None,
            purl: None,
        }
    }
}

/// One detected vulnerability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vulnerability {
    pub id: String,
    pub pkg_name: String,
    pub installed_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_version: Option<String>,
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl Vulnerability {
    pub fn new(id: &str, pkg: &str, ver: &str, sev: Severity) -> Self {
        Self {
            id: id.into(),
            pkg_name: pkg.into(),
            installed_version: ver.into(),
            fixed_version: None,
            severity: sev,
            references: Vec::new(),
            title: None,
        }
    }
}

/// One detected misconfiguration (Terraform/K8s/Docker/Helm rule hit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Misconfiguration {
    pub id: String,
    pub r#type: String,
    pub title: String,
    pub description: String,
    pub severity: Severity,
    pub resource: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
}

/// One detected secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret {
    pub rule_id: String,
    pub category: String,
    pub severity: Severity,
    pub start_line: u32,
    pub end_line: u32,
    pub match_text: String,
    pub file: String,
}

/// One license finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct License {
    pub pkg_name: String,
    pub license: String,
    pub category: LicenseCategory,
    pub confidence: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LicenseCategory {
    Permissive,
    Weakcopyleft,
    Copyleft,
    NetworkCopyleft,
    Restricted,
    Forbidden,
    Unknown,
}

/// Per-target result block (vulns + misconfigs + secrets + licenses).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScanResult {
    pub target: String,
    pub class: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vulnerabilities: Vec<Vulnerability>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub misconfigurations: Vec<Misconfiguration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<Secret>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub licenses: Vec<License>,
}

/// Top-level scan report — equivalent to trivy's `types.Report`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub artifact_name: String,
    pub artifact_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<DetectedOs>,
    #[serde(default)]
    pub results: Vec<ScanResult>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedOs {
    pub family: OsFamily,
    pub name: String,
}

impl Report {
    pub fn new(name: &str, kind: &str) -> Self {
        Self {
            schema_version: 2,
            artifact_name: name.into(),
            artifact_type: kind.into(),
            os: None,
            results: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn total_vulns(&self) -> usize {
        self.results.iter().map(|r| r.vulnerabilities.len()).sum()
    }
    pub fn total_misconfigs(&self) -> usize {
        self.results.iter().map(|r| r.misconfigurations.len()).sum()
    }
    pub fn total_secrets(&self) -> usize {
        self.results.iter().map(|r| r.secrets.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_target_str() {
        for (t, s) in [
            (ScanTarget::Image, "image"),
            (ScanTarget::Filesystem, "fs"),
            (ScanTarget::Repo, "repo"),
            (ScanTarget::K8s, "k8s"),
            (ScanTarget::Sbom, "sbom"),
            (ScanTarget::Secret, "secret"),
            (ScanTarget::Config, "config"),
        ] {
            assert_eq!(t.as_str(), s);
        }
    }

    #[test]
    fn os_family_from_id() {
        assert_eq!(OsFamily::from_id("Alpine"), OsFamily::Alpine);
        assert_eq!(OsFamily::from_id("DEBIAN"), OsFamily::Debian);
        assert_eq!(OsFamily::from_id("ubuntu"), OsFamily::Ubuntu);
        assert_eq!(OsFamily::from_id("redhat"), OsFamily::Rhel);
        assert_eq!(OsFamily::from_id("rhel"), OsFamily::Rhel);
        assert_eq!(OsFamily::from_id("amzn"), OsFamily::Amazon);
        assert_eq!(OsFamily::from_id("ol"), OsFamily::Oracle);
        assert_eq!(OsFamily::from_id("sles"), OsFamily::Suse);
        assert_eq!(OsFamily::from_id("opensuse"), OsFamily::OpenSuse);
        assert_eq!(OsFamily::from_id("rocky"), OsFamily::Rocky);
        assert_eq!(OsFamily::from_id("almalinux"), OsFamily::Alma);
        assert_eq!(OsFamily::from_id("photon"), OsFamily::Photon);
        assert_eq!(OsFamily::from_id("mariner"), OsFamily::Mariner);
        assert_eq!(OsFamily::from_id("haiku"), OsFamily::Unknown);
    }

    #[test]
    fn package_new_defaults() {
        let p = Package::new("openssl", "3.0.0", "alpine");
        assert_eq!(p.name, "openssl");
        assert_eq!(p.version, "3.0.0");
        assert!(p.source.is_none());
        assert!(p.purl.is_none());
    }

    #[test]
    fn vulnerability_new() {
        let v = Vulnerability::new("CVE-2026-1", "ssl", "1.0", Severity::High);
        assert_eq!(v.id, "CVE-2026-1");
        assert_eq!(v.severity, Severity::High);
        assert!(v.fixed_version.is_none());
    }

    #[test]
    fn report_totals() {
        let mut r = Report::new("alpine:3.19", "container_image");
        let mut sr = ScanResult {
            target: "alpine:3.19".into(),
            class: "os-pkgs".into(),
            ..Default::default()
        };
        sr.vulnerabilities
            .push(Vulnerability::new("CVE-2026-1", "x", "1", Severity::Low));
        sr.vulnerabilities
            .push(Vulnerability::new("CVE-2026-2", "y", "2", Severity::High));
        sr.secrets.push(Secret {
            rule_id: "gh-pat".into(),
            category: "aws".into(),
            severity: Severity::Critical,
            start_line: 1,
            end_line: 1,
            match_text: "ghp_…".into(),
            file: "f".into(),
        });
        r.results.push(sr);
        assert_eq!(r.total_vulns(), 2);
        assert_eq!(r.total_misconfigs(), 0);
        assert_eq!(r.total_secrets(), 1);
    }

    #[test]
    fn report_serializes_round_trip() {
        let r = Report::new("nginx:1.27", "container_image");
        let j = serde_json::to_string(&r).unwrap();
        let back: Report = serde_json::from_str(&j).unwrap();
        assert_eq!(back.artifact_name, "nginx:1.27");
        assert_eq!(back.schema_version, 2);
    }

    #[test]
    fn license_category_serde() {
        let c = LicenseCategory::Copyleft;
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(s, "\"copyleft\"");
    }
}
