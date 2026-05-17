// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: sonatype/nexus-public@HEAD components/nexus-repository/.../Repository.java + Component.java + Asset.java
//! Sonatype Nexus 3 domain model.
//!
//! Mirrors Nexus' public REST shapes for repositories, components, assets,
//! cleanup policies, and routing rules. Field names follow the upstream
//! `nexus.repository.*` Java/Groovy DTOs so an HTTP client written against
//! Nexus can target this surface with minor base-path changes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Content format an artifact belongs to. Mirrors Nexus' `Format` enum.
///
/// Only [`Format::Raw`] is end-to-end implemented in this initial port;
/// the remaining variants are recognised so manifests, repository definitions
/// and routing rules can reference them, but their dedicated upload/parse
/// adapters land in follow-up work tracked under the parity manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Raw,
    Maven2,
    Npm,
    Docker,
    PyPI,
    NuGet,
    Helm,
    Apt,
    Yum,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Raw => "raw",
            Format::Maven2 => "maven2",
            Format::Npm => "npm",
            Format::Docker => "docker",
            Format::PyPI => "pypi",
            Format::NuGet => "nuget",
            Format::Helm => "helm",
            Format::Apt => "apt",
            Format::Yum => "yum",
        }
    }

    pub fn parse(s: &str) -> Option<Format> {
        match s.to_ascii_lowercase().as_str() {
            "raw" => Some(Format::Raw),
            "maven2" | "maven" => Some(Format::Maven2),
            "npm" => Some(Format::Npm),
            "docker" => Some(Format::Docker),
            "pypi" => Some(Format::PyPI),
            "nuget" => Some(Format::NuGet),
            "helm" => Some(Format::Helm),
            "apt" => Some(Format::Apt),
            "yum" => Some(Format::Yum),
            _ => None,
        }
    }
}

/// Type of a repository: where the content comes from and how it is served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RepositoryType {
    /// Receives uploads from clients; canonical home of the content.
    Hosted {
        /// Whether re-uploading the same path is allowed (`allow`),
        /// only allowed for non-immutable assets (`allow_once`), or never
        /// allowed (`deny`).
        write_policy: WritePolicy,
    },
    /// Caches and proxies an upstream remote.
    Proxy {
        remote_url: String,
        /// Cache TTL in minutes. `0` disables caching entirely (always fetch).
        cache_ttl_minutes: u32,
    },
    /// Aggregates several repositories under one URL; resolves in member
    /// order, returning the first hit.
    Group {
        /// Member repository names, resolved in order.
        member_names: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritePolicy {
    Allow,
    AllowOnce,
    Deny,
}

/// A repository definition: name, format, type, online status, plus
/// optional cleanup-policy attachments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: Uuid,
    pub name: String,
    pub format: Format,
    #[serde(flatten)]
    pub repo_type: RepositoryType,
    pub online: bool,
    /// Names of cleanup policies attached to this repository (evaluated
    /// in order during cleanup runs).
    pub cleanup_policies: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// HTTP request body for `POST /api/nexus/v1/repositories`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub format: Format,
    #[serde(flatten)]
    pub repo_type: RepositoryType,
    #[serde(default = "default_true")]
    pub online: bool,
    #[serde(default)]
    pub cleanup_policies: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// HTTP request body for `PUT /api/nexus/v1/repositories/{name}`.
///
/// All fields optional: only those provided are updated, mirroring Nexus'
/// PATCH-like semantics under a PUT verb.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateRepositoryRequest {
    pub online: Option<bool>,
    pub cleanup_policies: Option<Vec<String>>,
    pub remote_url: Option<String>,
    pub cache_ttl_minutes: Option<u32>,
    pub member_names: Option<Vec<String>>,
    pub write_policy: Option<WritePolicy>,
}

/// Logical grouping of related assets that share a coordinate (group/name/
/// version triple, in upstream Maven/PyPI parlance). For raw repos the
/// `group` part is the directory path and `version` is unused.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub format: Format,
    pub group: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Logical coordinate the format adapter extracts from a path. Equivalent
/// to Nexus' `Coordinates` interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentCoord {
    pub group: Option<String>,
    pub name: String,
    pub version: Option<String>,
}

/// Physical artifact bytes belonging to a component, identified by a path
/// inside the repository and stored as a content-addressable blob reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: Uuid,
    pub component_id: Uuid,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub path: String,
    pub blob: BlobRef,
    pub content_type: String,
    pub created_at: DateTime<Utc>,
    pub last_modified: DateTime<Utc>,
    pub last_downloaded: Option<DateTime<Utc>>,
    pub download_count: u64,
}

/// Content-addressable handle to the bytes backing one or more assets.
/// Multiple assets may dedupe to the same blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobRef {
    pub sha256: String,
    pub size: u64,
}

/// Cleanup policy: deletes assets matching all configured criteria when run
/// against a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupPolicy {
    pub id: Uuid,
    pub name: String,
    pub format: Option<Format>,
    pub criteria: CleanupCriteria,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanupCriteria {
    /// Delete assets older than N days (by `created_at`).
    pub older_than_days: Option<u32>,
    /// Delete assets not downloaded within N days. Assets that have never
    /// been downloaded match if `created_at` exceeds the threshold.
    pub last_downloaded_days: Option<u32>,
    /// Regex applied to the asset path; only matching assets are eligible
    /// for deletion. When unset, all paths are eligible.
    pub regex: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateCleanupPolicyRequest {
    pub name: String,
    pub format: Option<Format>,
    pub criteria: CleanupCriteria,
}

/// Routing rule: governs whether request paths against a repository are
/// permitted. Used by Nexus to keep proxy repos from leaking unintended
/// paths to upstream remotes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    pub id: Uuid,
    pub name: String,
    pub mode: RoutingMode,
    pub matchers: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingMode {
    /// The request matches a routing rule iff one of the matchers matches;
    /// matched requests are explicitly allowed and denied otherwise.
    Allow,
    /// The request matches a routing rule iff one of the matchers matches;
    /// matched requests are explicitly blocked and allowed otherwise.
    Block,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRoutingRuleRequest {
    pub name: String,
    pub mode: RoutingMode,
    pub matchers: Vec<String>,
}

/// Outcome of a routing-rule evaluation: returned by the test endpoint and
/// consulted before serving any request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingDecision {
    Allowed,
    Blocked,
}

/// Server-side error envelope, modeled on Nexus' v1 problem JSON.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub details: HashMap<String, String>,
}

impl ErrorBody {
    pub fn new(error: &str, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
            details: HashMap::new(),
        }
    }

    pub fn with_detail(mut self, key: &str, val: impl Into<String>) -> Self {
        self.details.insert(key.into(), val.into());
        self
    }
}
