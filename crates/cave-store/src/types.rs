// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bucket {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub region: String,
    pub versioning: VersioningState,
    pub policy: Option<BucketPolicy>,
    pub acl: CannedAcl,
    pub lifecycle_rules: Vec<LifecycleRule>,
    pub notification_config: Option<NotificationConfig>,
    pub objects: HashMap<String, Vec<ObjectVersion>>, // key -> versions (latest first)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectVersion {
    pub version_id: String,
    pub is_latest: bool,
    pub is_delete_marker: bool,
    pub size: u64,
    pub etag: String,
    pub content_type: String,
    pub metadata: HashMap<String, String>,
    pub last_modified: DateTime<Utc>,
    pub data: Vec<u8>,
    pub encryption: Option<EncryptionInfo>,
    pub storage_class: StorageClass,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum VersioningState {
    Disabled,
    Enabled,
    Suspended,
}

impl Default for VersioningState {
    fn default() -> Self {
        Self::Disabled
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum CannedAcl {
    #[default]
    Private,
    PublicRead,
    PublicReadWrite,
    AuthenticatedRead,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BucketPolicy {
    pub version: String,
    pub statements: Vec<PolicyStatement>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyStatement {
    pub effect: String, // "Allow" | "Deny"
    pub principal: Vec<String>,
    pub action: Vec<String>,
    pub resource: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LifecycleRule {
    pub id: String,
    pub prefix: String,
    pub enabled: bool,
    pub expiration_days: Option<u32>,
    pub transition_days: Option<u32>,
    pub transition_storage_class: Option<StorageClass>,
    pub noncurrent_expiration_days: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum StorageClass {
    #[default]
    Standard,
    IA,
    Glacier,
    DeepArchive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncryptionInfo {
    pub algorithm: EncryptionAlgorithm,
    pub key_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EncryptionAlgorithm {
    SseS3,
    SseC,
    SseKms,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub queue_configurations: Vec<QueueConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueConfig {
    pub id: String,
    pub queue_arn: String,
    pub events: Vec<String>, // "s3:ObjectCreated:*", etc.
    pub prefix_filter: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MultipartUpload {
    pub upload_id: String,
    pub bucket: String,
    pub key: String,
    pub parts: std::collections::BTreeMap<u32, UploadPart>,
    pub initiated_at: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct UploadPart {
    pub part_number: u32,
    pub etag: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub bucket: String,
    pub key: String,
    pub version_id: Option<String>,
    pub size: u64,
    pub etag: String,
    pub content_type: String,
    pub last_modified: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
    pub storage_class: StorageClass,
}
