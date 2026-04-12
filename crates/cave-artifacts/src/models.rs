//! Core data models for cave-artifacts (Pulp-compatible).
//!
//! Workflow: Repository → Content → Publication → Distribution

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Plugin / content type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PluginType {
    File,
    Python,
    Rpm,
    Deb,
    Container,
    Ansible,
    Maven,
}

impl PluginType {
    pub fn api_segment(&self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Python => "python",
            Self::Rpm => "rpm",
            Self::Deb => "deb",
            Self::Container => "container",
            Self::Ansible => "ansible",
            Self::Maven => "maven",
        }
    }
}

impl std::fmt::Display for PluginType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.api_segment())
    }
}

// ---------------------------------------------------------------------------
// Download policy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadPolicy {
    /// Download all content during sync.
    Immediate,
    /// Download content on first client request.
    OnDemand,
    /// Stream from upstream without caching.
    Streamed,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        Self::Immediate
    }
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Waiting,
    Skipped,
    Running,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskError {
    pub description: String,
    pub traceback: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub state: TaskState,
    pub name: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<TaskError>,
    pub worker: Option<String>,
    /// HREFs of resources created by this task.
    pub created_resources: Vec<String>,
    /// HREFs of resources this task holds locks on.
    pub reserved_resources: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl Task {
    pub fn new(name: impl Into<String>, reserved: Vec<String>) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/tasks/{}/", id),
            pulp_id: id,
            state: TaskState::Waiting,
            name: name.into(),
            started_at: None,
            finished_at: None,
            error: None,
            worker: None,
            created_resources: vec![],
            reserved_resources: reserved,
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Repository + version
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub plugin_type: PluginType,
    pub description: Option<String>,
    pub latest_version_href: Option<String>,
    pub versions_href: String,
    /// Number of old versions to retain (None = keep all).
    pub retained_versions: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Repository {
    pub fn new(name: impl Into<String>, plugin_type: PluginType) -> Self {
        let id = Uuid::new_v4();
        let seg = plugin_type.api_segment();
        let href = format!("/pulp/api/v3/repositories/{seg}/{seg}/{id}/");
        Self {
            versions_href: format!("{href}versions/"),
            pulp_href: href,
            pulp_id: id,
            name: name.into(),
            plugin_type,
            description: None,
            latest_version_href: None,
            retained_versions: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentSummary {
    pub added: HashMap<String, u64>,
    pub removed: HashMap<String, u64>,
    pub present: HashMap<String, u64>,
}

impl ContentSummary {
    pub fn empty() -> Self {
        Self {
            added: HashMap::new(),
            removed: HashMap::new(),
            present: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryVersion {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub number: u64,
    pub repository: String,
    pub content_summary: ContentSummary,
    pub content_hrefs: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl RepositoryVersion {
    pub fn new(repo_href: &str, number: u64) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("{repo_href}versions/{number}/"),
            pulp_id: id,
            number,
            repository: repo_href.to_string(),
            content_summary: ContentSummary::empty(),
            content_hrefs: vec![],
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Remote
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Remote {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub plugin_type: PluginType,
    pub url: String,
    pub download_policy: DownloadPolicy,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls_validation: bool,
    pub proxy_url: Option<String>,
    pub ca_cert: Option<String>,
    pub client_cert: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Remote {
    pub fn new(name: impl Into<String>, plugin_type: PluginType, url: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        let seg = plugin_type.api_segment();
        Self {
            pulp_href: format!("/pulp/api/v3/remotes/{seg}/{seg}/{id}/"),
            pulp_id: id,
            name: name.into(),
            plugin_type,
            url: url.into(),
            download_policy: DownloadPolicy::Immediate,
            username: None,
            password: None,
            tls_validation: true,
            proxy_url: None,
            ca_cert: None,
            client_cert: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Content unit
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentUnit {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub plugin_type: PluginType,
    pub artifact_href: Option<String>,
    pub relative_path: Option<String>,
    pub sha256: Option<String>,
    pub size: Option<u64>,
    /// Plugin-specific metadata (package name, version, arch, etc.).
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl ContentUnit {
    pub fn new(plugin_type: PluginType, metadata: serde_json::Value) -> Self {
        let id = Uuid::new_v4();
        let seg = plugin_type.api_segment();
        Self {
            pulp_href: format!("/pulp/api/v3/content/{seg}/{seg}s/{id}/"),
            pulp_id: id,
            plugin_type,
            artifact_href: None,
            relative_path: None,
            sha256: None,
            size: None,
            metadata,
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Artifact (stored blob)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    #[serde(skip)]
    pub data: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

impl Artifact {
    pub fn new(data: Vec<u8>, sha256: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        let sha256 = sha256.into();
        let size = data.len() as u64;
        Self {
            pulp_href: format!("/pulp/api/v3/artifacts/{id}/"),
            pulp_id: id,
            file: sha256.clone(),
            size,
            sha256,
            md5: None,
            sha1: None,
            sha512: None,
            data,
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Publication
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Publication {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub plugin_type: PluginType,
    pub repository_version: String,
    pub repository: Option<String>,
    pub distributions: Vec<String>,
    pub signing_service: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Publication {
    pub fn new(plugin_type: PluginType, repo_version_href: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        let seg = plugin_type.api_segment();
        Self {
            pulp_href: format!("/pulp/api/v3/publications/{seg}/{seg}/{id}/"),
            pulp_id: id,
            plugin_type,
            repository_version: repo_version_href.into(),
            repository: None,
            distributions: vec![],
            signing_service: None,
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Distribution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distribution {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub plugin_type: PluginType,
    pub base_path: String,
    pub base_url: String,
    pub publication: Option<String>,
    pub repository: Option<String>,
    pub repository_version: Option<String>,
    pub content_guard: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Distribution {
    pub fn new(
        name: impl Into<String>,
        plugin_type: PluginType,
        base_path: impl Into<String>,
    ) -> Self {
        let id = Uuid::new_v4();
        let seg = plugin_type.api_segment();
        let base_path = base_path.into();
        Self {
            pulp_href: format!("/pulp/api/v3/distributions/{seg}/{seg}/{id}/"),
            pulp_id: id,
            name: name.into(),
            plugin_type,
            base_url: format!("https://artifacts.cave.io/pulp/content/{base_path}/"),
            base_path,
            publication: None,
            repository: None,
            repository_version: None,
            content_guard: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Content guard
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContentGuardType {
    Rbac,
    ContentRedirect,
    Header,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentGuard {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub guard_type: ContentGuardType,
    pub description: Option<String>,
    pub header_name: Option<String>,
    pub header_value: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ContentGuard {
    pub fn new(name: impl Into<String>, guard_type: ContentGuardType) -> Self {
        let id = Uuid::new_v4();
        let type_seg = match &guard_type {
            ContentGuardType::Rbac => "rbac",
            ContentGuardType::ContentRedirect => "content_redirect",
            ContentGuardType::Header => "header",
        };
        Self {
            pulp_href: format!("/pulp/api/v3/content-guards/{type_seg}/{id}/"),
            pulp_id: id,
            name: name.into(),
            guard_type,
            description: None,
            header_name: None,
            header_value: None,
            created_at: Utc::now(),
        }
    }

    /// Returns true if the request is allowed through this guard.
    pub fn allows(&self, headers: &HashMap<String, String>) -> bool {
        match self.guard_type {
            ContentGuardType::Rbac => true, // RBAC handled by cave-auth layer
            ContentGuardType::ContentRedirect => true,
            ContentGuardType::Header => {
                let Some(ref name) = self.header_name else { return true };
                let Some(ref value) = self.header_value else { return true };
                headers.get(name).map(|v| v == value).unwrap_or(false)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Signing service
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningService {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub public_key: String,
    pub pubkey_fingerprint: String,
    pub script: String,
    pub created_at: DateTime<Utc>,
}

impl SigningService {
    pub fn new(name: impl Into<String>, public_key: impl Into<String>, script: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/signing-services/{id}/"),
            pulp_id: id,
            name: name.into(),
            public_key: public_key.into(),
            pubkey_fingerprint: String::new(),
            script: script.into(),
            created_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Exporter / Export
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exporter {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub path: String,
    pub repositories: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl Exporter {
    pub fn new(name: impl Into<String>, path: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/exporters/core/pulp/{id}/"),
            pulp_id: id,
            name: name.into(),
            path: path.into(),
            repositories: vec![],
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Export {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub task: String,
    pub exported_resources: Vec<String>,
    pub output_file_info: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Request / response DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub plugin_type: PluginType,
    pub description: Option<String>,
    pub retained_versions: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRemoteRequest {
    pub name: String,
    pub plugin_type: PluginType,
    pub url: String,
    pub download_policy: Option<DownloadPolicy>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls_validation: Option<bool>,
    pub proxy_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SyncRequest {
    pub remote: Option<String>,
    pub mirror: Option<bool>,
    pub optimize: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePublicationRequest {
    pub repository_version: Option<String>,
    pub repository: Option<String>,
    pub signing_service: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDistributionRequest {
    pub name: String,
    pub plugin_type: PluginType,
    pub base_path: String,
    pub publication: Option<String>,
    pub repository: Option<String>,
    pub content_guard: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddContentRequest {
    pub content_units: Vec<String>,
    pub base_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RemoveContentRequest {
    pub content_units: Vec<String>,
    pub base_version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PulpPage<T: Serialize> {
    pub count: usize,
    pub next: Option<String>,
    pub previous: Option<String>,
    pub results: Vec<T>,
}

impl<T: Serialize> PulpPage<T> {
    pub fn of(results: Vec<T>) -> Self {
        Self {
            count: results.len(),
            next: None,
            previous: None,
            results,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AsyncOperationResponse {
    pub task: String,
}
