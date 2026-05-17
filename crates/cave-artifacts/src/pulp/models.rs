// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: pulp/pulpcore@0f991c2fa2bf6c8635e8a2de064ef04dacbbcf4f pulpcore/app/serializers/repository.py
//! Pulp v3 data models.
//!
//! Repository, RepositoryVersion, Remote, Distribution, Publication,
//! ContentGuard, all content types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Content type enum ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Rpm,
    Debian,
    Python,
    Container,
    File,
    Ansible,
    Maven,
    Gem,
    Npm,
    Generic,
}

impl ContentType {
    pub fn plugin_name(&self) -> &'static str {
        match self {
            Self::Rpm => "pulp_rpm",
            Self::Debian => "pulp_deb",
            Self::Python => "pulp_python",
            Self::Container => "pulp_container",
            Self::File => "pulp_file",
            Self::Ansible => "pulp_ansible",
            Self::Maven => "pulp_maven",
            Self::Gem => "pulp_gem",
            Self::Npm => "pulp_npm",
            Self::Generic => "pulp_core",
        }
    }
}

// ─── Repository ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub pulp_last_updated: DateTime<Utc>,
    pub name: String,
    pub description: Option<String>,
    pub content_type: ContentType,
    pub retain_repo_versions: Option<u32>,
    /// Latest version href.
    pub latest_version_href: Option<String>,
    /// Versions count.
    pub versions_href: String,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub autopublish: bool,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

impl Repository {
    pub fn new(name: impl Into<String>, content_type: ContentType) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/repositories/{}/", id),
            pulp_id: id,
            pulp_created: Utc::now(),
            pulp_last_updated: Utc::now(),
            name: name.into(),
            description: None,
            content_type,
            retain_repo_versions: Some(10),
            latest_version_href: None,
            versions_href: format!("/pulp/api/v3/repositories/{}/versions/", id),
            remote: None,
            autopublish: false,
            labels: HashMap::new(),
        }
    }
}

// ─── RepositoryVersion ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryVersion {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub number: u64,
    pub repository: String,
    pub base_version: Option<String>,
    pub content_summary: ContentSummary,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContentSummary {
    pub added: HashMap<String, ContentCount>,
    pub removed: HashMap<String, ContentCount>,
    pub present: HashMap<String, ContentCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentCount {
    pub count: u64,
    pub href: String,
}

impl RepositoryVersion {
    pub fn new(repo_href: &str, number: u64) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("{}{}/", repo_href.trim_end_matches('/').replace("repositories", "versions"), number),
            pulp_id: id,
            pulp_created: Utc::now(),
            number,
            repository: repo_href.to_string(),
            base_version: None,
            content_summary: ContentSummary::default(),
            complete: false,
        }
    }
}

// ─── Remote ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub pulp_last_updated: DateTime<Utc>,
    pub name: String,
    pub url: String,
    pub content_type: ContentType,
    pub ca_cert: Option<String>,
    pub client_cert: Option<String>,
    pub tls_validation: bool,
    pub proxy_url: Option<String>,
    pub username: Option<String>,
    /// Token (sensitive, write-only).
    pub token: Option<String>,
    pub download_concurrency: Option<u32>,
    pub max_retries: Option<u32>,
    pub policy: RemotePolicy,
    pub total_timeout: Option<f64>,
    pub connect_timeout: Option<f64>,
    pub headers: Vec<HashMap<String, String>>,
    pub rate_limit: Option<u32>,
    /// Remote-type-specific options.
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RemotePolicy {
    /// Download content when syncing.
    Immediate,
    /// Download content on demand.
    OnDemand,
    /// Download only metadata; stream content on demand.
    Streamed,
}

impl Default for RemotePolicy {
    fn default() -> Self {
        Self::Immediate
    }
}

impl Remote {
    pub fn new(name: impl Into<String>, url: impl Into<String>, content_type: ContentType) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/remotes/{}/", id),
            pulp_id: id,
            pulp_created: Utc::now(),
            pulp_last_updated: Utc::now(),
            name: name.into(),
            url: url.into(),
            content_type,
            ca_cert: None,
            client_cert: None,
            tls_validation: true,
            proxy_url: None,
            username: None,
            token: None,
            download_concurrency: Some(4),
            max_retries: Some(3),
            policy: RemotePolicy::default(),
            total_timeout: None,
            connect_timeout: None,
            headers: vec![],
            rate_limit: None,
            extra: HashMap::new(),
        }
    }
}

// ─── Publication ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Publication {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub repository_version: String,
    pub content_type: ContentType,
    pub complete: bool,
    /// Content-type-specific metadata.
    pub extra: HashMap<String, serde_json::Value>,
}

impl Publication {
    pub fn new(repo_version_href: impl Into<String>, content_type: ContentType) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/publications/{}/", id),
            pulp_id: id,
            pulp_created: Utc::now(),
            repository_version: repo_version_href.into(),
            content_type,
            complete: false,
            extra: HashMap::new(),
        }
    }
}

// ─── Distribution ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub pulp_last_updated: DateTime<Utc>,
    pub name: String,
    pub base_path: String,
    pub base_url: String,
    pub content_type: ContentType,
    pub publication: Option<String>,
    pub repository: Option<String>,
    pub repository_version: Option<String>,
    pub content_guard: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

impl Distribution {
    pub fn new(name: impl Into<String>, base_path: impl Into<String>, content_type: ContentType) -> Self {
        let id = Uuid::new_v4();
        let bp = base_path.into();
        Self {
            pulp_href: format!("/pulp/api/v3/distributions/{}/", id),
            pulp_id: id,
            pulp_created: Utc::now(),
            pulp_last_updated: Utc::now(),
            name: name.into(),
            base_url: format!("/pulp/content/{}/", bp),
            base_path: bp,
            content_type,
            publication: None,
            repository: None,
            repository_version: None,
            content_guard: None,
            hidden: false,
            labels: HashMap::new(),
        }
    }
}

// ─── ContentGuard ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentGuard {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub name: String,
    pub description: Option<String>,
    pub guard_type: ContentGuardType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ContentGuardType {
    /// Require RBAC for content access.
    Rbac,
    /// Require a specific header value.
    Header { header_name: String, header_value: String },
    /// Require X.509 client certificate.
    X509 { ca_certificate: String },
    /// Content URL signing (pre-signed URLs).
    ContentRedirect,
    /// Compound guard: all must pass.
    Composite { guards: Vec<String> },
}

// ─── Content types ────────────────────────────────────────────────────────────

/// Generic content artifact (file-based).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub file: String,
    pub size: u64,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha224: Option<String>,
    pub sha256: Option<String>,
    pub sha384: Option<String>,
    pub sha512: Option<String>,
    pub timestamp_of_interest: Option<DateTime<Utc>>,
}

/// RPM package content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpmPackage {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub version: String,
    pub release: String,
    pub arch: String,
    pub epoch: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub rpm_license: Option<String>,
    pub rpm_vendor: Option<String>,
    pub rpm_group: Option<String>,
    pub source_rpm: Option<String>,
    pub artifact: String,
    pub location_href: String,
    pub sha256: String,
    pub size_package: u64,
    pub time_file: u64,
    pub time_build: u64,
}

impl RpmPackage {
    pub fn nevra(&self) -> String {
        format!("{}-{}-{}.{}.rpm", self.name, self.version, self.release, self.arch)
    }
}

/// Debian package content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebPackage {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub package: String,
    pub version: String,
    pub architecture: String,
    pub section: Option<String>,
    pub priority: Option<String>,
    pub maintainer: Option<String>,
    pub description: Option<String>,
    pub depends: Option<String>,
    pub pre_depends: Option<String>,
    pub suggests: Option<String>,
    pub recommends: Option<String>,
    pub sha256: String,
    pub size: u64,
    pub artifact: String,
    pub relative_path: String,
}

/// Python package (PyPI) content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PythonPackage {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub version: String,
    pub filename: String,
    pub packagetype: PythonPackageType,
    pub python_version: Option<String>,
    pub requires_python: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub sha256: String,
    pub artifact: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PythonPackageType {
    Sdist,
    Bdist_wheel,
    Bdist_egg,
}

/// OCI/Container image.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerManifest {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub digest: String,
    pub schema_version: u32,
    pub media_type: String,
    pub labels: HashMap<String, String>,
    pub is_bootable: bool,
    pub is_flatpak: bool,
    pub artifact: String,
}

/// Container image tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerTag {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub tagged_manifest: String,
    pub repository_version: Option<String>,
}

/// Ansible collection content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnsibleCollection {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub artifact: String,
    pub requires_ansible: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub dependencies: HashMap<String, String>,
}

/// Maven artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MavenArtifact {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    pub filename: String,
    pub artifact: String,
    pub sha256: String,
    pub relative_path: String,
}

impl MavenArtifact {
    pub fn coordinates(&self) -> String {
        format!("{}:{}:{}", self.group_id, self.artifact_id, self.version)
    }
}

/// Generic file content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub relative_path: String,
    pub artifact: String,
    pub sha256: String,
}

// ─── Sync result ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub added: u64,
    pub removed: u64,
    pub unchanged: u64,
    pub total_size_bytes: u64,
    pub duration_seconds: f64,
    pub new_version_href: Option<String>,
}

// ─── Import/Export ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PulpExport {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub exporter: String,
    pub params: ExportParams,
    pub task: Option<String>,
    pub output_file_info: Option<serde_json::Value>,
    pub toc_info: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportParams {
    pub repositories: Vec<String>,
    pub versions: Vec<String>,
    pub chunk_size: Option<u64>,
    pub start_versions: Vec<String>,
    pub full: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PulpImport {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub importer: String,
    pub params: ImportParams,
    pub task: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportParams {
    pub path: String,
    pub toc: Option<String>,
    pub create_repositories: bool,
}

// ─── RBAC ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub permissions: Vec<String>,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRole {
    pub pulp_href: String,
    pub role: String,
    pub content_object: Option<String>,
    pub domain: Option<String>,
}

// ─── Paginated response ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub count: u64,
    pub next: Option<String>,
    pub previous: Option<String>,
    pub results: Vec<T>,
}

impl<T> PaginatedResponse<T> {
    pub fn of(results: Vec<T>) -> Self {
        Self {
            count: results.len() as u64,
            next: None,
            previous: None,
            results,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_new_sets_hrefs() {
        let repo = Repository::new("my-rpm-repo", ContentType::Rpm);
        assert!(repo.pulp_href.starts_with("/pulp/api/v3/repositories/"));
        assert!(repo.versions_href.contains("versions"));
        assert_eq!(repo.content_type, ContentType::Rpm);
        assert_eq!(repo.retain_repo_versions, Some(10));
    }

    #[test]
    fn content_type_plugin_names() {
        assert_eq!(ContentType::Rpm.plugin_name(), "pulp_rpm");
        assert_eq!(ContentType::Debian.plugin_name(), "pulp_deb");
        assert_eq!(ContentType::Python.plugin_name(), "pulp_python");
        assert_eq!(ContentType::Container.plugin_name(), "pulp_container");
        assert_eq!(ContentType::Ansible.plugin_name(), "pulp_ansible");
        assert_eq!(ContentType::Maven.plugin_name(), "pulp_maven");
    }

    #[test]
    fn rpm_package_nevra() {
        let rpm = RpmPackage {
            pulp_href: "/pulp/api/v3/content/rpm/packages/abc/".to_string(),
            pulp_id: Uuid::new_v4(),
            name: "httpd".to_string(),
            version: "2.4.57".to_string(),
            release: "1.el9".to_string(),
            arch: "x86_64".to_string(),
            epoch: "0".to_string(),
            summary: None,
            description: None,
            url: None,
            rpm_license: None,
            rpm_vendor: None,
            rpm_group: None,
            source_rpm: None,
            artifact: "/pulp/api/v3/artifacts/def/".to_string(),
            location_href: "Packages/h/httpd-2.4.57-1.el9.x86_64.rpm".to_string(),
            sha256: "abc123".to_string(),
            size_package: 1024 * 1024,
            time_file: 1700000000,
            time_build: 1699000000,
        };
        assert_eq!(rpm.nevra(), "httpd-2.4.57-1.el9.x86_64.rpm");
    }

    #[test]
    fn maven_artifact_coordinates() {
        let artifact = MavenArtifact {
            pulp_href: "/pulp/api/v3/content/maven/artifacts/abc/".to_string(),
            pulp_id: Uuid::new_v4(),
            group_id: "com.example".to_string(),
            artifact_id: "my-lib".to_string(),
            version: "1.2.3".to_string(),
            filename: "my-lib-1.2.3.jar".to_string(),
            artifact: "/pulp/api/v3/artifacts/def/".to_string(),
            sha256: "abc123".to_string(),
            relative_path: "com/example/my-lib/1.2.3/my-lib-1.2.3.jar".to_string(),
        };
        assert_eq!(artifact.coordinates(), "com.example:my-lib:1.2.3");
    }

    #[test]
    fn distribution_base_url() {
        let dist = Distribution::new("my-dist", "pypi/simple", ContentType::Python);
        assert!(dist.base_url.contains("pypi/simple"));
    }

    #[test]
    fn remote_policy_default() {
        assert_eq!(RemotePolicy::default(), RemotePolicy::Immediate);
    }

    #[test]
    fn paginated_response_count() {
        let resp = PaginatedResponse::of(vec![1, 2, 3]);
        assert_eq!(resp.count, 3);
        assert!(resp.next.is_none());
    }

    #[test]
    fn repository_version_new() {
        let repo_href = "/pulp/api/v3/repositories/abc/";
        let ver = RepositoryVersion::new(repo_href, 5);
        assert_eq!(ver.number, 5);
        assert!(!ver.complete);
    }

    #[test]
    fn remote_tls_validation_default() {
        let remote = Remote::new("upstream", "https://pypi.org/simple/", ContentType::Python);
        assert!(remote.tls_validation);
        assert_eq!(remote.policy, RemotePolicy::Immediate);
    }
}
