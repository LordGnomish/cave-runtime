// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Manifest and registry types: OCI, Docker V2 Schema 2, Manifest List.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Media types ──────────────────────────────────────────────────────────────

pub const MEDIA_MANIFEST_V2: &str =
    "application/vnd.docker.distribution.manifest.v2+json";
pub const MEDIA_MANIFEST_LIST: &str =
    "application/vnd.docker.distribution.manifest.list.v2+json";
pub const MEDIA_OCI_MANIFEST: &str =
    "application/vnd.oci.image.manifest.v1+json";
pub const MEDIA_OCI_INDEX: &str =
    "application/vnd.oci.image.index.v1+json";
pub const MEDIA_LAYER_GZIP: &str =
    "application/vnd.docker.image.rootfs.diff.tar.gzip";
pub const MEDIA_CONFIG: &str =
    "application/vnd.docker.container.image.v1+json";

// ── Wire types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: String,
    pub size: i64,
    pub digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(rename = "os.version", skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
}

/// Docker Image Manifest V2 Schema 2 / OCI Image Manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageManifest {
    pub schema_version: i32,
    pub media_type: String,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

/// Docker Manifest List / OCI Image Index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestList {
    pub schema_version: i32,
    pub media_type: String,
    pub manifests: Vec<Descriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

// ── Internal storage types ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StoredManifest {
    pub digest: String,
    pub media_type: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct StoredBlob {
    pub digest: String,
    pub size: u64,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct UploadSession {
    pub session_id: String,
    pub repository: String,
    pub data: Vec<u8>,
    pub offset: u64,
}

// ── Policy / access types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct TagPolicy {
    /// Specific tags that are immutable (exact match).
    pub immutable_tags: Vec<String>,
    /// If true, ALL tags in this repository are immutable.
    pub all_immutable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    Pull,
    Push,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessRule {
    pub subject: String,
    pub permission: Permission,
}

// ── Webhook types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub id: String,
    pub repository: Option<String>, // None = global
    pub url: String,
    pub events: Vec<WebhookEvent>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEvent {
    Push,
    Pull,
    Delete,
    ScanComplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub event: WebhookEvent,
    pub repository: String,
    pub digest: Option<String>,
    pub tag: Option<String>,
    pub timestamp: String,
}

// ── Replication types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationTarget {
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub username: Option<String>,
    pub password: Option<String>,
}

// ── Scan types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub manifest_digest: String,
    pub scanner: String,
    pub status: ScanStatus,
    pub vulnerabilities: Vec<Vulnerability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ScanStatus {
    Pending,
    Scanning,
    Complete,
    Failed,
    NotSupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vulnerability {
    pub id: String,
    pub severity: String,
    pub package: String,
    pub version: String,
    pub fixed_version: Option<String>,
    pub description: Option<String>,
}

// ── Catalog / tag list response types ────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogResponse {
    pub repositories: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TagsListResponse {
    pub name: String,
    pub tags: Vec<String>,
}
