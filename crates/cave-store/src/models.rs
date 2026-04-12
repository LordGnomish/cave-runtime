//! Data models for cave-store.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// An object-storage bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bucket {
    pub id: Uuid,
    pub name: String,
    pub region: String,
    pub access_policy: AccessPolicy,
    pub versioning_enabled: bool,
    pub lifecycle_rules: Vec<LifecycleRule>,
    pub replication_rules: Vec<ReplicationRule>,
    pub tags: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
}

/// A stored object (one version).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageObject {
    pub key: String,
    pub bucket: String,
    pub size: u64,
    pub content_type: String,
    pub metadata: HashMap<String, String>,
    /// Hex digest (simplified: UUID-based).
    pub etag: String,
    /// Present when versioning is enabled on the bucket.
    pub version_id: Option<Uuid>,
    /// True for versioning delete markers.
    pub is_delete_marker: bool,
    pub last_modified: DateTime<Utc>,
    /// Object body stored as JSON value (binary data sent as base64 string).
    pub content: serde_json::Value,
}

/// Bucket-level access policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessPolicy {
    pub public_read: bool,
    pub public_write: bool,
    pub allowed_origins: Vec<String>,
    /// Raw policy document (S3-compatible JSON).
    pub policy_document: Option<serde_json::Value>,
}

impl Default for AccessPolicy {
    fn default() -> Self {
        Self {
            public_read: false,
            public_write: false,
            allowed_origins: vec![],
            policy_document: None,
        }
    }
}

/// A lifecycle rule that expires or transitions objects automatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleRule {
    pub id: Uuid,
    /// Key prefix this rule applies to (`""` = all objects).
    pub prefix: String,
    /// Delete objects after this many days.
    pub expiration_days: Option<u32>,
    /// Move to this storage class after `expiration_days`.
    pub transition_storage_class: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// Cross-bucket replication rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationRule {
    pub id: Uuid,
    pub destination_bucket: String,
    /// Only replicate objects whose key starts with this prefix.
    pub prefix: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// In-progress multipart upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultipartUpload {
    pub upload_id: Uuid,
    pub bucket: String,
    pub key: String,
    pub parts: Vec<UploadPart>,
    pub initiated_at: DateTime<Utc>,
    pub content_type: String,
    pub metadata: HashMap<String, String>,
}

/// A single part within a multipart upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadPart {
    pub part_number: u32,
    pub etag: String,
    pub size: u64,
    pub content: serde_json::Value,
}
