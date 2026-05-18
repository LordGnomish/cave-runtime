// SPDX-License-Identifier: AGPL-3.0-or-later
//! Container image vulnerability scanner (Trivy replacement).
//!
//! In production this module would fetch OCI image layers, parse package
//! manifests, and query a live CVE database. Here we expose the core
//! data structures and matching logic so it can be driven by either a
//! real feed or the built-in sample data.

use crate::models::{
    CveEntry, CvssSeverity, PolicyResult, SbomComponent, SbomDocument, SbomFormat, ScanPolicy,
    ScanResult, Vulnerability,
};
use chrono::Utc;
use uuid::Uuid;

/// A package installed inside an image layer.
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    /// SHA-256 digest of the layer that introduced this package.
    pub layer_digest: Option<String>,
}

/// Run a vulnerability scan for `image_ref`.
///
/// `installed_packages` represents the packages found by walking OCI layers.
/// `db` is the CVE database to check against.
/// `policy` controls which findings cause a policy failure.
pub fn scan_image(
    image_ref: &str,
    image_digest: &str,
    db: &[CveEntry],
    installed_packages: &[InstalledPackage],
    policy: &ScanPolicy,
) -> ScanResult {
    let vulnerabilities = find_vulnerabilities(installed_packages, db);
    let policy_result = evaluate_policy(&vulnerabilities, policy);

    ScanResult {
        id: Uuid::new_v4(),
        image_reference: image_ref.to_string(),
        image_digest: image_digest.to_string(),
        scanned_at: Utc::now(),
        vulnerabilities,
        policy_result,
        signature_verified: false,
    }
}

/// Match installed packages against the CVE database.
pub fn find_vulnerabilities(packages: &[InstalledPackage], db: &[CveEntry]) -> Vec<Vulnerability> {
    let mut vulns = Vec::new();
    for pkg in packages {
        for cve in db {
            if cve.affected_package == pkg.name
                && cve.affected_versions.iter().any(|v| v == &pkg.version)
            {
                vulns.push(Vulnerability {
                    id: Uuid::new_v4(),
                    cve_id: cve.cve_id.clone(),
                    package_name: pkg.name.clone(),
                    installed_version: pkg.version.clone(),
                    fixed_version: cve.fixed_version.clone(),
                    severity: cve.severity,
                    cvss_score: cve.cvss_score,
                    description: cve.description.clone(),
                    layer_digest: pkg.layer_digest.clone(),
                });
            }
        }
    }
    vulns
}

/// Apply the scan policy to a list of vulnerabilities.
pub fn evaluate_policy(vulns: &[Vulnerability], policy: &ScanPolicy) -> PolicyResult {
    if !policy.enabled {
        return PolicyResult::Pass;
    }

    let violations: Vec<String> = vulns
        .iter()
        .filter(|v| !policy.allowed_cves.contains(&v.cve_id))
        .filter(|v| v.severity >= policy.fail_on_severity)
        .map(|v| {
            format!(
                "{} in {}@{} — {}",
                v.cve_id, v.package_name, v.installed_version, v.severity
            )
        })
        .collect();

    if violations.is_empty() {
        PolicyResult::Pass
    } else {
        PolicyResult::Fail { reasons: violations }
    }
}

/// Generate an SBOM document for an image in the requested format.
pub fn generate_sbom(
    image_ref: &str,
    packages: &[InstalledPackage],
    format: SbomFormat,
) -> SbomDocument {
    let components = packages
        .iter()
        .map(|p| SbomComponent {
            name: p.name.clone(),
            version: p.version.clone(),
            purl: format!("pkg:generic/{}@{}", p.name, p.version),
            licenses: Vec::new(),
            supplier: None,
            checksum_sha256: None,
        })
        .collect();

    SbomDocument {
        id: Uuid::new_v4(),
        format,
        created_at: Utc::now(),
        image_reference: image_ref.to_string(),
        components,
    }
}

/// Sample CVE database for tests and demo mode.
pub fn sample_cve_db() -> Vec<CveEntry> {
    vec![
        CveEntry {
            cve_id: "CVE-2023-1234".to_string(),
            description: "Buffer overflow in libssl allows remote code execution".to_string(),
            severity: CvssSeverity::Critical,
            cvss_score: 9.8,
            affected_package: "openssl".to_string(),
            affected_versions: vec!["1.1.1".to_string(), "1.1.0".to_string()],
            fixed_version: Some("1.1.2".to_string()),
            published_at: Utc::now(),
            references: vec![
                "https://nvd.nist.gov/vuln/detail/CVE-2023-1234".to_string(),
            ],
        },
        CveEntry {
            cve_id: "CVE-2023-5678".to_string(),
            description: "Privilege escalation in sudo via crafted command".to_string(),
            severity: CvssSeverity::High,
            cvss_score: 7.8,
            affected_package: "sudo".to_string(),
            affected_versions: vec!["1.9.5".to_string(), "1.9.4".to_string()],
            fixed_version: Some("1.9.6".to_string()),
            published_at: Utc::now(),
            references: Vec::new(),
        },
        CveEntry {
            cve_id: "CVE-2023-9012".to_string(),
            description: "Information disclosure in curl via malformed URL".to_string(),
            severity: CvssSeverity::Medium,
            cvss_score: 5.3,
            affected_package: "curl".to_string(),
            affected_versions: vec!["7.84.0".to_string()],
            fixed_version: Some("7.85.0".to_string()),
            published_at: Utc::now(),
            references: Vec::new(),
        },
    ]
}
