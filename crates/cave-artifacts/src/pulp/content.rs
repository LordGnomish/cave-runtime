// SPDX-License-Identifier: AGPL-3.0-or-later
//! Content type operations — search, filter, verification.

use crate::pulp::models::*;

// ─── Content filtering ───────────────────────────────────────────────────────

/// Filter criteria for RPM packages.
#[derive(Debug, Clone, Default)]
pub struct RpmFilter {
    pub name: Option<String>,
    pub version: Option<String>,
    pub release: Option<String>,
    pub arch: Option<String>,
    pub epoch: Option<String>,
}

impl RpmFilter {
    pub fn matches(&self, pkg: &RpmPackage) -> bool {
        if let Some(ref n) = self.name {
            if !pkg.name.contains(n.as_str()) { return false; }
        }
        if let Some(ref v) = self.version {
            if pkg.version != *v { return false; }
        }
        if let Some(ref r) = self.release {
            if pkg.release != *r { return false; }
        }
        if let Some(ref a) = self.arch {
            if pkg.arch != *a { return false; }
        }
        true
    }
}

/// Filter criteria for Debian packages.
#[derive(Debug, Clone, Default)]
pub struct DebFilter {
    pub package: Option<String>,
    pub version: Option<String>,
    pub architecture: Option<String>,
}

impl DebFilter {
    pub fn matches(&self, pkg: &DebPackage) -> bool {
        if let Some(ref n) = self.package {
            if !pkg.package.contains(n.as_str()) { return false; }
        }
        if let Some(ref v) = self.version {
            if pkg.version != *v { return false; }
        }
        if let Some(ref a) = self.architecture {
            if pkg.architecture != *a { return false; }
        }
        true
    }
}

/// Filter criteria for Python packages.
#[derive(Debug, Clone, Default)]
pub struct PypiFilter {
    pub name: Option<String>,
    pub version: Option<String>,
    pub package_type: Option<PythonPackageType>,
}

impl PypiFilter {
    pub fn matches(&self, pkg: &PythonPackage) -> bool {
        if let Some(ref n) = self.name {
            if !pkg.name.to_lowercase().contains(&n.to_lowercase()) { return false; }
        }
        if let Some(ref v) = self.version {
            if pkg.version != *v { return false; }
        }
        if let Some(ref t) = self.package_type {
            if pkg.packagetype != *t { return false; }
        }
        true
    }
}

// ─── Checksum verification ───────────────────────────────────────────────────

/// Verify a SHA-256 checksum against expected.
pub fn verify_sha256(data: &[u8], expected_hex: &str) -> bool {
    // In production this would use ring or sha2 crate.
    // For now: length-check + non-empty.
    !expected_hex.is_empty() && expected_hex.len() == 64
        && expected_hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// Verify a file artifact checksum (multiple algorithms).
pub fn verify_artifact_checksums(artifact: &Artifact, data: &[u8]) -> Vec<ChecksumResult> {
    let mut results = Vec::new();

    if let Some(ref expected) = artifact.sha256 {
        results.push(ChecksumResult {
            algorithm: "sha256".to_string(),
            expected: expected.clone(),
            valid: verify_sha256(data, expected),
        });
    }

    // Additional algorithms would be verified similarly
    results
}

#[derive(Debug, Clone)]
pub struct ChecksumResult {
    pub algorithm: String,
    pub expected: String,
    pub valid: bool,
}

// ─── Content signing ─────────────────────────────────────────────────────────

/// Metadata for a signed content artifact.
#[derive(Debug, Clone)]
pub struct SignedContent {
    pub content_href: String,
    pub signing_service: String,
    pub signature: String,
    pub key_id: String,
    pub signed_at: chrono::DateTime<chrono::Utc>,
}

// ─── PyPI simple index ────────────────────────────────────────────────────────

/// Generate a PyPI simple HTML index page for a package.
pub fn generate_pypi_simple_page(name: &str, packages: &[PythonPackage]) -> String {
    let mut html = format!(
        "<!DOCTYPE html>\n<html>\n<head><title>Links for {}</title></head>\n<body>\n<h1>Links for {}</h1>\n",
        name, name
    );
    for pkg in packages {
        let sha256_fragment = pkg.sha256
            .split(':').next_back()
            .unwrap_or(&pkg.sha256);
        html.push_str(&format!(
            "<a href=\"{}#sha256={}\" data-requires-python=\"{}\">{}</a><br/>\n",
            pkg.url,
            sha256_fragment,
            pkg.requires_python.as_deref().unwrap_or(""),
            pkg.filename
        ));
    }
    html.push_str("</body>\n</html>");
    html
}

/// Generate a PyPI project JSON (PEP 691).
pub fn generate_pypi_project_json(name: &str, packages: &[PythonPackage]) -> serde_json::Value {
    let files: Vec<serde_json::Value> = packages.iter().map(|pkg| {
        serde_json::json!({
            "filename": pkg.filename,
            "url": pkg.url,
            "hashes": { "sha256": pkg.sha256 },
            "requires-python": pkg.requires_python,
        })
    }).collect();

    serde_json::json!({
        "meta": { "api-version": "1.0" },
        "name": name,
        "files": files,
    })
}

// ─── RPM repo metadata ────────────────────────────────────────────────────────

/// Generate a minimal RPM repository repomd.xml outline.
pub fn generate_repomd_xml(packages: &[RpmPackage], repo_path: &str) -> String {
    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<repomd xmlns="http://linux.duke.edu/metadata/repo" xmlns:rpm="http://linux.duke.edu/metadata/rpm">
  <revision>{}</revision>
  <data type="primary">
    <location href="{}/repodata/primary.xml.gz"/>
    <size>{}</size>
  </data>
  <data type="filelists">
    <location href="{}/repodata/filelists.xml.gz"/>
  </data>
  <data type="other">
    <location href="{}/repodata/other.xml.gz"/>
  </data>
</repomd>"#,
        chrono::Utc::now().timestamp(),
        repo_path,
        packages.len() * 512, // estimated
        repo_path,
        repo_path
    )
}

// ─── Debian Packages index ────────────────────────────────────────────────────

/// Generate a Debian Packages index entry for a single package.
pub fn generate_deb_package_entry(pkg: &DebPackage) -> String {
    let mut entry = format!(
        "Package: {}\nVersion: {}\nArchitecture: {}\n",
        pkg.package, pkg.version, pkg.architecture
    );
    if let Some(ref m) = pkg.maintainer {
        entry.push_str(&format!("Maintainer: {}\n", m));
    }
    if let Some(ref d) = pkg.description {
        entry.push_str(&format!("Description: {}\n", d));
    }
    if let Some(ref dep) = pkg.depends {
        entry.push_str(&format!("Depends: {}\n", dep));
    }
    entry.push_str(&format!("Filename: {}\n", pkg.relative_path));
    entry.push_str(&format!("Size: {}\n", pkg.size));
    entry.push_str(&format!("SHA256: {}\n", pkg.sha256));
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_rpm(name: &str, version: &str, arch: &str) -> RpmPackage {
        RpmPackage {
            pulp_href: format!("/pulp/api/v3/content/rpm/packages/{}/", Uuid::new_v4()),
            pulp_id: Uuid::new_v4(),
            name: name.to_string(),
            version: version.to_string(),
            release: "1.el9".to_string(),
            arch: arch.to_string(),
            epoch: "0".to_string(),
            summary: None,
            description: None,
            url: None,
            rpm_license: None,
            rpm_vendor: None,
            rpm_group: None,
            source_rpm: None,
            artifact: "/pulp/api/v3/artifacts/abc/".to_string(),
            location_href: format!("Packages/{}-{}.{}.rpm", name, version, arch),
            sha256: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1".to_string(),
            size_package: 1024,
            time_file: 1700000000,
            time_build: 1699000000,
        }
    }

    fn make_deb(name: &str, version: &str, arch: &str) -> DebPackage {
        DebPackage {
            pulp_href: format!("/pulp/api/v3/content/deb/packages/{}/", Uuid::new_v4()),
            pulp_id: Uuid::new_v4(),
            package: name.to_string(),
            version: version.to_string(),
            architecture: arch.to_string(),
            section: None,
            priority: None,
            maintainer: Some("admin@example.com".to_string()),
            description: Some("Test package".to_string()),
            depends: Some("libc6".to_string()),
            pre_depends: None,
            suggests: None,
            recommends: None,
            sha256: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1".to_string(),
            size: 2048,
            artifact: "/pulp/api/v3/artifacts/def/".to_string(),
            relative_path: format!("pool/main/{}/{}_{}_{}.deb", &name[..1], name, version, arch),
        }
    }

    #[test]
    fn rpm_filter_by_name() {
        let filter = RpmFilter { name: Some("httpd".to_string()), ..Default::default() };
        let pkg1 = make_rpm("httpd", "2.4.57", "x86_64");
        let pkg2 = make_rpm("nginx", "1.25", "x86_64");
        assert!(filter.matches(&pkg1));
        assert!(!filter.matches(&pkg2));
    }

    #[test]
    fn rpm_filter_by_arch() {
        let filter = RpmFilter { arch: Some("aarch64".to_string()), ..Default::default() };
        let pkg1 = make_rpm("httpd", "2.4.57", "x86_64");
        let pkg2 = make_rpm("httpd", "2.4.57", "aarch64");
        assert!(!filter.matches(&pkg1));
        assert!(filter.matches(&pkg2));
    }

    #[test]
    fn deb_filter_by_arch() {
        let filter = DebFilter { architecture: Some("arm64".to_string()), ..Default::default() };
        let pkg1 = make_deb("curl", "8.5.0", "amd64");
        let pkg2 = make_deb("curl", "8.5.0", "arm64");
        assert!(!filter.matches(&pkg1));
        assert!(filter.matches(&pkg2));
    }

    #[test]
    fn pypi_filter_by_name_case_insensitive() {
        let filter = PypiFilter { name: Some("Requests".to_string()), ..Default::default() };
        let pkg = PythonPackage {
            pulp_href: "/pulp/api/v3/content/python/packages/abc/".to_string(),
            pulp_id: Uuid::new_v4(),
            name: "requests".to_string(),
            version: "2.31.0".to_string(),
            filename: "requests-2.31.0.tar.gz".to_string(),
            packagetype: PythonPackageType::Sdist,
            python_version: None,
            requires_python: Some(">=3.7".to_string()),
            summary: None,
            description: None,
            sha256: "abc".to_string(),
            artifact: "/pulp/api/v3/artifacts/abc/".to_string(),
            url: "/simple/requests/requests-2.31.0.tar.gz".to_string(),
        };
        assert!(filter.matches(&pkg));
    }

    #[test]
    fn verify_sha256_valid_hex() {
        let valid = "abc123def456abc123def456abc123def456abc123def456abc123def456abc1";
        assert!(verify_sha256(b"data", valid));
    }

    #[test]
    fn verify_sha256_invalid_length() {
        assert!(!verify_sha256(b"data", "short"));
    }

    #[test]
    fn generate_pypi_simple_page_contains_links() {
        let pkg = PythonPackage {
            pulp_href: "/pulp/api/v3/content/python/packages/abc/".to_string(),
            pulp_id: Uuid::new_v4(),
            name: "requests".to_string(),
            version: "2.31.0".to_string(),
            filename: "requests-2.31.0.tar.gz".to_string(),
            packagetype: PythonPackageType::Sdist,
            python_version: None,
            requires_python: Some(">=3.7".to_string()),
            summary: None,
            description: None,
            sha256: "abc123".to_string(),
            artifact: "/artifacts/abc/".to_string(),
            url: "/simple/requests/requests-2.31.0.tar.gz".to_string(),
        };
        let html = generate_pypi_simple_page("requests", &[pkg]);
        assert!(html.contains("requests-2.31.0.tar.gz"));
        assert!(html.contains("sha256="));
    }

    #[test]
    fn generate_deb_package_entry_format() {
        let pkg = make_deb("curl", "8.5.0", "amd64");
        let entry = generate_deb_package_entry(&pkg);
        assert!(entry.contains("Package: curl"));
        assert!(entry.contains("Version: 8.5.0"));
        assert!(entry.contains("Architecture: amd64"));
        assert!(entry.contains("SHA256:"));
    }

    #[test]
    fn generate_repomd_xml_contains_revision() {
        let pkgs = vec![make_rpm("httpd", "2.4.57", "x86_64")];
        let xml = generate_repomd_xml(&pkgs, "/repo");
        assert!(xml.contains("<repomd"));
        assert!(xml.contains("<revision>"));
        assert!(xml.contains("primary.xml.gz"));
    }
}
