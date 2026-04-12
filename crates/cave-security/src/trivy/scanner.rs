//! Core scanner — orchestrates OS packages, language packages, secrets,
//! misconfigurations, and SBOM generation for image and filesystem targets.

use crate::trivy::{
    ignore::TrivyIgnore,
    lang_pkg::{self, LangPackage},
    license::{self, LicenseFinding},
    misconfig::{self, MisconfigFinding},
    os_pkg::{self, OsPackage},
    sbom::{self, CycloneDxBom},
    secret::{self, SecretFinding, SecretSeverity},
    vuln_db::{Severity, VulnDb},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
};

// ---------------------------------------------------------------------------
// Scan target / options
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanType {
    Image,
    Filesystem,
    Config,
    Sbom,
}

impl std::fmt::Display for ScanType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanType::Image => write!(f, "image"),
            ScanType::Filesystem => write!(f, "filesystem"),
            ScanType::Config => write!(f, "config"),
            ScanType::Sbom => write!(f, "sbom"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanOptions {
    /// Minimum severity to include (defaults to all).
    pub min_severity: Option<Severity>,
    /// Path to .trivyignore file content.
    pub ignore_content: Option<String>,
    /// Scan for secrets.
    #[serde(default = "default_true")]
    pub scan_secrets: bool,
    /// Scan for misconfigurations.
    #[serde(default = "default_true")]
    pub scan_misconfig: bool,
    /// Generate SBOM.
    #[serde(default = "default_true")]
    pub generate_sbom: bool,
    /// Output format.
    pub output_format: Option<crate::trivy::output::OutputFormat>,
}

fn default_true() -> bool { true }

// ---------------------------------------------------------------------------
// Scan result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnFinding {
    pub cve_id: String,
    pub title: Option<String>,
    pub package_name: String,
    pub installed_version: String,
    pub fixed_version: Option<String>,
    pub severity: Severity,
    pub ecosystem: String,
    pub description: Option<String>,
}

impl SecretFinding {
    pub fn severity_str(&self) -> &str {
        match self.severity {
            SecretSeverity::Critical => "CRITICAL",
            SecretSeverity::High => "HIGH",
            SecretSeverity::Medium => "MEDIUM",
            SecretSeverity::Low => "LOW",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub target: String,
    pub scan_type: ScanType,
    pub vulnerabilities: Vec<VulnFinding>,
    pub secrets: Vec<SecretFinding>,
    pub licenses: Vec<LicenseFinding>,
    pub misconfigs: Vec<MisconfigFinding>,
    pub sbom: Option<CycloneDxBom>,
    pub scanned_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Scanner
// ---------------------------------------------------------------------------

pub struct Scanner<'a> {
    pub vuln_db: &'a VulnDb,
    pub secret_patterns: Vec<secret::SecretPattern>,
}

impl<'a> Scanner<'a> {
    pub fn new(vuln_db: &'a VulnDb) -> Self {
        Scanner {
            vuln_db,
            secret_patterns: secret::builtin_patterns(),
        }
    }

    // -----------------------------------------------------------------------
    // Image scan
    // -----------------------------------------------------------------------

    /// Scan a container image filesystem (pre-extracted directory path).
    ///
    /// For tarball scanning pass the directory where the image has been
    /// extracted. The scanner looks for well-known package-database paths.
    pub fn scan_image_dir(
        &self,
        image_ref: &str,
        root_dir: &Path,
        opts: &ScanOptions,
    ) -> ScanResult {
        let ignore = opts
            .ignore_content
            .as_deref()
            .map(TrivyIgnore::parse)
            .unwrap_or_default();

        let (os_pkgs, lang_pkgs) = self.collect_packages(root_dir);
        let vulns = self.match_vulns(&os_pkgs, &lang_pkgs, &ignore, opts.min_severity);
        let secrets = if opts.scan_secrets {
            self.scan_secrets_in_dir(root_dir)
        } else {
            vec![]
        };
        let licenses = self.collect_licenses(&os_pkgs);
        let sbom = if opts.generate_sbom {
            Some(sbom::generate_cyclonedx(image_ref, &os_pkgs, &lang_pkgs))
        } else {
            None
        };

        ScanResult {
            target: image_ref.to_string(),
            scan_type: ScanType::Image,
            vulnerabilities: vulns,
            secrets,
            licenses,
            misconfigs: vec![],
            sbom,
            scanned_at: Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Filesystem scan
    // -----------------------------------------------------------------------

    /// Scan a local directory for packages, secrets, and misconfigurations.
    pub fn scan_filesystem(
        &self,
        path: &Path,
        opts: &ScanOptions,
    ) -> ScanResult {
        let ignore = opts
            .ignore_content
            .as_deref()
            .map(TrivyIgnore::parse)
            .unwrap_or_default();

        let (os_pkgs, lang_pkgs) = self.collect_packages(path);
        let vulns = self.match_vulns(&os_pkgs, &lang_pkgs, &ignore, opts.min_severity);
        let secrets = if opts.scan_secrets {
            self.scan_secrets_in_dir(path)
        } else {
            vec![]
        };
        let misconfigs = if opts.scan_misconfig {
            self.scan_misconfigs_in_dir(path)
        } else {
            vec![]
        };
        let licenses = self.collect_licenses(&os_pkgs);
        let sbom = if opts.generate_sbom {
            Some(sbom::generate_cyclonedx(
                path.to_string_lossy().as_ref(),
                &os_pkgs,
                &lang_pkgs,
            ))
        } else {
            None
        };

        ScanResult {
            target: path.to_string_lossy().to_string(),
            scan_type: ScanType::Filesystem,
            vulnerabilities: vulns,
            secrets,
            licenses,
            misconfigs,
            sbom,
            scanned_at: Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Config scan
    // -----------------------------------------------------------------------

    /// Scan config content (Dockerfile, K8s YAML, Terraform) for misconfigs.
    pub fn scan_config(
        &self,
        file_path: &str,
        content: &str,
    ) -> ScanResult {
        let misconfigs = if file_path.ends_with("Dockerfile") || file_path.contains("dockerfile") {
            misconfig::scan_dockerfile(content, file_path)
        } else if file_path.ends_with(".yaml") || file_path.ends_with(".yml") {
            misconfig::scan_k8s_yaml(content, file_path)
        } else if file_path.ends_with(".tf") {
            misconfig::scan_terraform(content, file_path)
        } else {
            // Try to auto-detect
            let lower = content.to_lowercase();
            if lower.contains("apiversion:") || lower.contains("kind:") {
                misconfig::scan_k8s_yaml(content, file_path)
            } else if lower.starts_with("from ") || lower.contains("\nfrom ") {
                misconfig::scan_dockerfile(content, file_path)
            } else {
                vec![]
            }
        };

        ScanResult {
            target: file_path.to_string(),
            scan_type: ScanType::Config,
            vulnerabilities: vec![],
            secrets: vec![],
            licenses: vec![],
            misconfigs,
            sbom: None,
            scanned_at: Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn collect_packages(&self, root: &Path) -> (Vec<OsPackage>, Vec<LangPackage>) {
        let mut os_pkgs = Vec::new();
        let mut lang_pkgs = Vec::new();

        // OS packages
        let apk_path = root.join("lib/apk/db/installed");
        if let Ok(content) = std::fs::read_to_string(&apk_path) {
            os_pkgs.extend(os_pkg::parse_apk_installed(&content));
        }
        let dpkg_path = root.join("var/lib/dpkg/status");
        if let Ok(content) = std::fs::read_to_string(&dpkg_path) {
            os_pkgs.extend(os_pkg::parse_dpkg_status(&content));
        }
        // RPM support: rpm query output file (if present)
        let rpm_path = root.join("var/lib/rpm/packages.txt");
        if let Ok(content) = std::fs::read_to_string(&rpm_path) {
            os_pkgs.extend(os_pkg::parse_rpm_query_output(&content));
        }

        // Language packages — walk the filesystem looking for manifest files
        self.walk_for_manifests(root, &mut lang_pkgs);

        (os_pkgs, lang_pkgs)
    }

    fn walk_for_manifests(&self, dir: &Path, pkgs: &mut Vec<LangPackage>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return; };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip common non-source directories
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if !matches!(name, ".git" | "node_modules" | "target" | ".cache" | "vendor") {
                    self.walk_for_manifests(&path, pkgs);
                }
            } else {
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                let path_str = path.to_string_lossy().to_string();
                if let Some(_eco) = lang_pkg::detect_manifest_type(file_name) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let found: Vec<LangPackage> = match file_name {
                            "go.sum" => lang_pkg::parse_go_sum(&content, &path_str),
                            "package-lock.json" => lang_pkg::parse_package_lock_json(&content, &path_str),
                            "requirements.txt" => lang_pkg::parse_requirements_txt(&content, &path_str),
                            "pom.xml" => lang_pkg::parse_pom_xml(&content, &path_str),
                            "Cargo.lock" => lang_pkg::parse_cargo_lock(&content, &path_str),
                            "composer.lock" => lang_pkg::parse_composer_lock(&content, &path_str),
                            _ => vec![],
                        };
                        pkgs.extend(found);
                    }
                }
            }
        }
    }

    fn match_vulns(
        &self,
        os_pkgs: &[OsPackage],
        lang_pkgs: &[LangPackage],
        ignore: &TrivyIgnore,
        min_severity: Option<Severity>,
    ) -> Vec<VulnFinding> {
        let mut findings = Vec::new();

        for pkg in os_pkgs {
            let eco = format!("os/{}", pkg.package_manager);
            let vulns = self.vuln_db.lookup_package(&eco, &pkg.name);
            for vuln in vulns {
                if ignore.is_ignored(&vuln.cve_id) { continue; }
                if let Some(min) = min_severity {
                    if vuln.severity < min { continue; }
                }
                findings.push(VulnFinding {
                    cve_id: vuln.cve_id.clone(),
                    title: Some(vuln.title.clone()),
                    package_name: pkg.name.clone(),
                    installed_version: pkg.version.clone(),
                    fixed_version: vuln.fixed_version.clone(),
                    severity: vuln.severity,
                    ecosystem: eco.clone(),
                    description: Some(vuln.description.clone()),
                });
            }
        }

        for pkg in lang_pkgs {
            let vulns = self.vuln_db.lookup_package(&pkg.ecosystem.to_string(), &pkg.name);
            for vuln in vulns {
                if ignore.is_ignored(&vuln.cve_id) { continue; }
                if let Some(min) = min_severity {
                    if vuln.severity < min { continue; }
                }
                findings.push(VulnFinding {
                    cve_id: vuln.cve_id.clone(),
                    title: Some(vuln.title.clone()),
                    package_name: pkg.name.clone(),
                    installed_version: pkg.version.clone(),
                    fixed_version: vuln.fixed_version.clone(),
                    severity: vuln.severity,
                    ecosystem: pkg.ecosystem.to_string(),
                    description: Some(vuln.description.clone()),
                });
            }
        }

        // Deduplicate by (cve_id, package_name)
        let mut seen = std::collections::HashSet::new();
        findings.retain(|f| seen.insert((f.cve_id.clone(), f.package_name.clone())));

        findings
    }

    fn scan_secrets_in_dir(&self, dir: &Path) -> Vec<SecretFinding> {
        let mut findings = Vec::new();
        let _ = self.walk_for_secrets(dir, &mut findings, 0);
        findings
    }

    fn walk_for_secrets(&self, dir: &Path, findings: &mut Vec<SecretFinding>, depth: usize) {
        if depth > 10 { return; }
        let Ok(entries) = std::fs::read_dir(dir) else { return; };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if !matches!(name, ".git" | "node_modules" | "target" | ".cache") {
                    self.walk_for_secrets(&path, findings, depth + 1);
                }
            } else if should_scan_for_secrets(&path) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let path_str = path.to_string_lossy().to_string();
                    findings.extend(secret::scan_file_for_secrets(
                        &content,
                        &path_str,
                        &self.secret_patterns,
                    ));
                }
            }
        }
    }

    fn scan_misconfigs_in_dir(&self, dir: &Path) -> Vec<MisconfigFinding> {
        let mut findings = Vec::new();
        self.walk_for_misconfigs(dir, &mut findings, 0);
        findings
    }

    fn walk_for_misconfigs(&self, dir: &Path, findings: &mut Vec<MisconfigFinding>, depth: usize) {
        if depth > 8 { return; }
        let Ok(entries) = std::fs::read_dir(dir) else { return; };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if !matches!(name, ".git" | "node_modules" | "target") {
                    self.walk_for_misconfigs(&path, findings, depth + 1);
                }
            } else {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                let path_str = path.to_string_lossy().to_string();
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if name == "Dockerfile" || name.starts_with("Dockerfile.") {
                        findings.extend(misconfig::scan_dockerfile(&content, &path_str));
                    } else if name.ends_with(".yaml") || name.ends_with(".yml") {
                        findings.extend(misconfig::scan_k8s_yaml(&content, &path_str));
                    } else if name.ends_with(".tf") {
                        findings.extend(misconfig::scan_terraform(&content, &path_str));
                    }
                }
            }
        }
    }

    fn collect_licenses(&self, os_pkgs: &[OsPackage]) -> Vec<LicenseFinding> {
        let mut findings = Vec::new();
        let mut seen: HashMap<String, bool> = HashMap::new();
        for pkg in os_pkgs {
            for lic in &pkg.licenses {
                if !seen.contains_key(lic.as_str()) {
                    seen.insert(lic.clone(), true);
                    findings.push(license::classify_license(lic, &pkg.name));
                }
            }
        }
        findings
    }
}

fn should_scan_for_secrets(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();

    // Skip binary-heavy extensions
    if matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "pdf" | "zip" | "tar" | "gz" | "bin" | "exe" | "so" | "dylib") {
        return false;
    }
    // Skip large compiled artifacts
    if matches!(ext, "class" | "jar" | "pyc" | "pyo") {
        return false;
    }
    // Always scan common secret-bearing files
    if matches!(name, ".env" | ".envrc" | "credentials" | "config" | "secrets") {
        return true;
    }
    // Scan text-like files by extension
    matches!(ext, "yaml" | "yml" | "json" | "toml" | "env" | "conf" | "config" | "txt" | "sh" | "py" | "rb" | "js" | "ts" | "go" | "rs" | "java" | "xml" | "properties" | "ini" | "cfg" | "tf" | "tfvars")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trivy::vuln_db::VulnDb;

    #[test]
    fn scan_config_dockerfile() {
        let db = VulnDb::default();
        let scanner = Scanner::new(&db);
        let dockerfile = "FROM ubuntu:latest\nRUN apt-get install curl\n";
        let result = scanner.scan_config("Dockerfile", dockerfile);
        assert_eq!(result.scan_type.to_string(), "config");
        assert!(!result.misconfigs.is_empty()); // no USER + latest tag
    }

    #[test]
    fn scan_config_k8s() {
        let db = VulnDb::default();
        let scanner = Scanner::new(&db);
        let yaml = r#"
apiVersion: v1
kind: Pod
spec:
  hostNetwork: true
  containers:
  - name: app
    image: app:1.0
"#;
        let result = scanner.scan_config("pod.yaml", yaml);
        assert!(!result.misconfigs.is_empty());
    }

    #[test]
    fn scan_config_terraform() {
        let db = VulnDb::default();
        let scanner = Scanner::new(&db);
        let tf = r#"resource "aws_security_group_rule" "open" {
  cidr_blocks = ["0.0.0.0/0"]
}"#;
        let result = scanner.scan_config("sg.tf", tf);
        assert!(!result.misconfigs.is_empty());
    }

    #[test]
    fn scan_empty_dir() {
        let db = VulnDb::default();
        let scanner = Scanner::new(&db);
        let dir = std::env::temp_dir();
        let opts = ScanOptions { generate_sbom: false, ..Default::default() };
        let result = scanner.scan_filesystem(&dir, &opts);
        // Should not panic; may have 0 or more findings depending on /tmp
        assert_eq!(result.scan_type.to_string(), "filesystem");
    }
}
