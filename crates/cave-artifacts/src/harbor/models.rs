// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/pkg/repository/model.go + src/pkg/artifact/model.go
//! OCI / Docker manifest and blob models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Media types ──────────────────────────────────────────────────────────────

pub const MEDIA_TYPE_MANIFEST_V2: &str =
    "application/vnd.docker.distribution.manifest.v2+json";
pub const MEDIA_TYPE_MANIFEST_LIST: &str =
    "application/vnd.docker.distribution.manifest.list.v2+json";
pub const MEDIA_TYPE_OCI_MANIFEST: &str =
    "application/vnd.oci.image.manifest.v1+json";
pub const MEDIA_TYPE_OCI_INDEX: &str =
    "application/vnd.oci.image.index.v1+json";
pub const MEDIA_TYPE_OCI_CONFIG: &str =
    "application/vnd.oci.image.config.v1+json";
pub const MEDIA_TYPE_OCI_LAYER_GZIP: &str =
    "application/vnd.oci.image.layer.v1.tar+gzip";
pub const MEDIA_TYPE_OCI_EMPTY: &str =
    "application/vnd.oci.empty.v1+json";
pub const MEDIA_TYPE_DOCKER_CONFIG: &str =
    "application/vnd.docker.container.image.v1+json";
pub const MEDIA_TYPE_DOCKER_LAYER: &str =
    "application/vnd.docker.image.rootfs.diff.tar.gzip";

// ── OCI Descriptor ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Descriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: i64,
    pub digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    #[serde(rename = "artifactType", skip_serializing_if = "Option::is_none")]
    pub artifact_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub urls: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(rename = "os.version", skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(rename = "os.features", skip_serializing_if = "Option::is_none")]
    pub os_features: Option<Vec<String>>,
}

// ── Image Manifest V2 / OCI Manifest ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
    /// OCI 1.1: pointer to the artifact this manifest describes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Descriptor>,
    /// OCI 1.1: artifact type
    #[serde(rename = "artifactType", skip_serializing_if = "Option::is_none")]
    pub artifact_type: Option<String>,
}

// ── Manifest List / OCI Image Index ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestList {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub manifests: Vec<Descriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Descriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

// ── OCI 1.1 Referrers ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferrersResponse {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub manifests: Vec<Descriptor>,
}

impl ReferrersResponse {
    pub fn new(manifests: Vec<Descriptor>) -> Self {
        Self {
            schema_version: 2,
            media_type: MEDIA_TYPE_OCI_INDEX.to_string(),
            manifests,
        }
    }
}

// ── Tags / Catalog ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagList {
    pub name: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub repositories: Vec<String>,
}

// ── Internal storage entries ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ManifestEntry {
    pub digest: String,
    pub content_type: String,
    pub data: bytes::Bytes,
    pub subject_digest: Option<String>,
    pub artifact_type: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UploadState {
    pub uuid: String,
    pub repository: String,
    pub data: Vec<u8>,
    pub started_at: DateTime<Utc>,
}

impl UploadState {
    pub fn new(uuid: String, repository: String) -> Self {
        Self {
            uuid,
            repository,
            data: Vec::new(),
            started_at: Utc::now(),
        }
    }

    pub fn offset(&self) -> usize {
        self.data.len()
    }
}

// ── OCI error codes and response ─────────────────────────────────────────────

pub const ERR_BLOB_UNKNOWN: &str = "BLOB_UNKNOWN";
pub const ERR_BLOB_UPLOAD_INVALID: &str = "BLOB_UPLOAD_INVALID";
pub const ERR_BLOB_UPLOAD_UNKNOWN: &str = "BLOB_UPLOAD_UNKNOWN";
pub const ERR_DIGEST_INVALID: &str = "DIGEST_INVALID";
pub const ERR_MANIFEST_BLOB_UNKNOWN: &str = "MANIFEST_BLOB_UNKNOWN";
pub const ERR_MANIFEST_INVALID: &str = "MANIFEST_INVALID";
pub const ERR_MANIFEST_UNKNOWN: &str = "MANIFEST_UNKNOWN";
pub const ERR_NAME_INVALID: &str = "NAME_INVALID";
pub const ERR_NAME_UNKNOWN: &str = "NAME_UNKNOWN";
pub const ERR_SIZE_INVALID: &str = "SIZE_INVALID";
pub const ERR_UNAUTHORIZED: &str = "UNAUTHORIZED";
pub const ERR_DENIED: &str = "DENIED";
pub const ERR_UNSUPPORTED: &str = "UNSUPPORTED";
pub const ERR_TOO_MANY_REQUESTS: &str = "TOOMANYREQUESTS";

#[derive(Debug, Serialize)]
pub struct RegistryErrors {
    pub errors: Vec<RegistryErrorDetail>,
}

#[derive(Debug, Serialize)]
pub struct RegistryErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl RegistryErrors {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            errors: vec![RegistryErrorDetail {
                code: code.to_string(),
                message: message.to_string(),
                detail: None,
            }],
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"errors":[{"code":"INTERNAL_ERROR","message":"serialization failed"}]}"#
                .to_string()
        })
    }
}

// ── Path classification ───────────────────────────────────────────────────────

#[derive(Debug)]
pub enum V2Op {
    TagsList(String),
    Manifest(String, String),
    Blob(String, String),
    BlobUploadInitiate(String),
    BlobUploadSession(String, String),
    Referrers(String, String),
    Unknown,
}

/// Parse a path under /v2/ into a typed operation.
/// Uses rfind so repo names with slashes (e.g. "library/ubuntu") work.
pub fn classify_v2_path(path: &str) -> V2Op {
    let path = path.trim_start_matches('/');

    if path.ends_with("/tags/list") {
        let name = &path[..path.len() - "/tags/list".len()];
        if !name.is_empty() {
            return V2Op::TagsList(name.to_string());
        }
    }

    // blobs/uploads must be checked before blobs (more specific)
    if let Some(pos) = path.rfind("/blobs/uploads/") {
        let name = &path[..pos];
        let rest = &path[pos + "/blobs/uploads/".len()..];
        if !name.is_empty() {
            if rest.is_empty() {
                return V2Op::BlobUploadInitiate(name.to_string());
            } else {
                return V2Op::BlobUploadSession(name.to_string(), rest.to_string());
            }
        }
    }

    if let Some(pos) = path.rfind("/manifests/") {
        let name = &path[..pos];
        let reference = &path[pos + "/manifests/".len()..];
        if !name.is_empty() && !reference.is_empty() {
            return V2Op::Manifest(name.to_string(), reference.to_string());
        }
    }

    if let Some(pos) = path.rfind("/blobs/") {
        let name = &path[..pos];
        let digest = &path[pos + "/blobs/".len()..];
        if !name.is_empty() && !digest.is_empty() {
            return V2Op::Blob(name.to_string(), digest.to_string());
        }
    }

    if let Some(pos) = path.rfind("/referrers/") {
        let name = &path[..pos];
        let digest = &path[pos + "/referrers/".len()..];
        if !name.is_empty() && !digest.is_empty() {
            return V2Op::Referrers(name.to_string(), digest.to_string());
        }
    }

    V2Op::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_tags_list() {
        let op = classify_v2_path("library/ubuntu/tags/list");
        assert!(matches!(op, V2Op::TagsList(n) if n == "library/ubuntu"));
    }

    #[test]
    fn classify_manifest_by_tag() {
        let op = classify_v2_path("myorg/myimage/manifests/latest");
        assert!(matches!(op, V2Op::Manifest(n, r) if n == "myorg/myimage" && r == "latest"));
    }

    #[test]
    fn classify_manifest_by_digest() {
        let op = classify_v2_path(
            "library/ubuntu/manifests/sha256:abc123",
        );
        assert!(matches!(op, V2Op::Manifest(n, r) if n == "library/ubuntu" && r == "sha256:abc123"));
    }

    #[test]
    fn classify_blob_get() {
        let op = classify_v2_path("library/ubuntu/blobs/sha256:deadbeef");
        assert!(matches!(op, V2Op::Blob(n, d) if n == "library/ubuntu" && d == "sha256:deadbeef"));
    }

    #[test]
    fn classify_blob_upload_initiate() {
        let op = classify_v2_path("myrepo/myimage/blobs/uploads/");
        assert!(matches!(op, V2Op::BlobUploadInitiate(n) if n == "myrepo/myimage"));
    }

    #[test]
    fn classify_blob_upload_session() {
        let op =
            classify_v2_path("myrepo/myimage/blobs/uploads/some-uuid-here");
        assert!(
            matches!(op, V2Op::BlobUploadSession(n, u) if n == "myrepo/myimage" && u == "some-uuid-here")
        );
    }

    #[test]
    fn classify_referrers() {
        let op = classify_v2_path("org/app/referrers/sha256:abc");
        assert!(matches!(op, V2Op::Referrers(n, d) if n == "org/app" && d == "sha256:abc"));
    }
}
