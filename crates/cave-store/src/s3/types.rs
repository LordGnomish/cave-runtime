//! S3/MinIO data types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Bucket ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bucket {
    pub name: String,
    pub region: String,
    pub owner: String,
    pub created_at: DateTime<Utc>,
    pub versioning: VersioningState,
    pub acl: BucketAcl,
    pub policy: Option<String>,         // JSON policy doc
    pub lifecycle_rules: Vec<LifecycleRule>,
    pub notification_config: NotificationConfiguration,
    pub encryption: Option<BucketEncryption>,
    pub tags: HashMap<String, String>,
    pub object_lock: bool,
}

impl Bucket {
    pub fn new(name: String, region: String, owner: String) -> Self {
        Self {
            name,
            region,
            owner,
            created_at: Utc::now(),
            versioning: VersioningState::Disabled,
            acl: BucketAcl::Private,
            policy: None,
            lifecycle_rules: Vec::new(),
            notification_config: NotificationConfiguration::default(),
            encryption: None,
            tags: HashMap::new(),
            object_lock: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum VersioningState {
    Disabled,
    Enabled,
    Suspended,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BucketAcl {
    Private,
    PublicRead,
    PublicReadWrite,
    AuthenticatedRead,
}

// ── Lifecycle ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleRule {
    pub id: String,
    pub status: String, // "Enabled" | "Disabled"
    pub prefix: String,
    pub tags: HashMap<String, String>,
    pub expiration: Option<Expiration>,
    pub transitions: Vec<Transition>,
    pub noncurrent_version_expiration: Option<NoncurrentVersionExpiration>,
    pub abort_incomplete_multipart_upload: Option<AbortIncompleteMultipartUpload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expiration {
    pub days: Option<u32>,
    pub date: Option<DateTime<Utc>>,
    pub expired_object_delete_marker: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    pub days: Option<u32>,
    pub date: Option<DateTime<Utc>>,
    pub storage_class: StorageClass,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoncurrentVersionExpiration {
    pub noncurrent_days: u32,
    pub newer_noncurrent_versions: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbortIncompleteMultipartUpload {
    pub days_after_initiation: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StorageClass {
    Standard,
    StandardIa,
    OnezoneIa,
    IntelligentTiering,
    Glacier,
    GlacierIr,
    DeepArchive,
    Outposts,
}

// ── Notification ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationConfiguration {
    pub queue_configurations: Vec<QueueConfiguration>,
    pub topic_configurations: Vec<TopicConfiguration>,
    pub lambda_function_configurations: Vec<LambdaFunctionConfiguration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfiguration {
    pub id: String,
    pub queue_arn: String,
    pub events: Vec<String>,
    pub filter: Option<NotificationFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicConfiguration {
    pub id: String,
    pub topic_arn: String,
    pub events: Vec<String>,
    pub filter: Option<NotificationFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LambdaFunctionConfiguration {
    pub id: String,
    pub lambda_function_arn: String,
    pub events: Vec<String>,
    pub filter: Option<NotificationFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationFilter {
    pub key: FilterKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterKey {
    pub filter_rules: Vec<FilterRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRule {
    pub name: String,  // "prefix" | "suffix"
    pub value: String,
}

// ── Encryption ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketEncryption {
    pub sse_algorithm: SseAlgorithm,
    pub kms_master_key_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SseAlgorithm {
    #[serde(rename = "AES256")]
    Aes256,
    #[serde(rename = "aws:kms")]
    AwsKms,
}

// ── Object ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectVersion {
    pub version_id: Option<String>, // None when versioning disabled
    pub etag: String,
    pub size: u64,
    pub last_modified: DateTime<Utc>,
    pub content_type: String,
    pub metadata: HashMap<String, String>,
    pub tags: HashMap<String, String>,
    pub storage_class: StorageClass,
    pub storage_path: String, // relative path on disk within data_dir
    pub encryption: Option<ObjectEncryption>,
    pub delete_marker: bool,
    pub restore_status: Option<RestoreStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectEncryption {
    pub algorithm: String, // "AES256" | "aws:kms" | "SSE-C"
    pub key_md5: Option<String>,
    pub kms_key_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreStatus {
    pub is_restore_in_progress: bool,
    pub restore_expiry_date: Option<DateTime<Utc>>,
}

// ── Multipart ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultipartUpload {
    pub upload_id: String,
    pub bucket: String,
    pub key: String,
    pub initiated: DateTime<Utc>,
    pub owner: String,
    pub content_type: String,
    pub metadata: HashMap<String, String>,
    pub parts: HashMap<u32, UploadedPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedPart {
    pub part_number: u32,
    pub etag: String,
    pub size: u64,
    pub storage_path: String,
    pub last_modified: DateTime<Utc>,
}

// ── Presigned ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PresignedMethod {
    Get,
    Put,
    Delete,
    Head,
}

// ── S3 Events ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Event {
    pub event_name: String, // e.g. "s3:ObjectCreated:Put"
    pub event_time: DateTime<Utc>,
    pub bucket: String,
    pub key: String,
    pub size: u64,
    pub etag: String,
    pub version_id: Option<String>,
    pub source_ip: Option<String>,
}

impl S3Event {
    pub fn object_created_put(bucket: &str, key: &str, size: u64, etag: &str) -> Self {
        Self {
            event_name: "s3:ObjectCreated:Put".to_string(),
            event_time: Utc::now(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            size,
            etag: etag.to_string(),
            version_id: None,
            source_ip: None,
        }
    }

    pub fn object_removed_delete(bucket: &str, key: &str) -> Self {
        Self {
            event_name: "s3:ObjectRemoved:Delete".to_string(),
            event_time: Utc::now(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            size: 0,
            etag: String::new(),
            version_id: None,
            source_ip: None,
        }
    }

    pub fn object_created_multipart(bucket: &str, key: &str, size: u64, etag: &str) -> Self {
        Self {
            event_name: "s3:ObjectCreated:CompleteMultipartUpload".to_string(),
            event_time: Utc::now(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            size,
            etag: etag.to_string(),
            version_id: None,
            source_ip: None,
        }
    }

    pub fn object_created_copy(bucket: &str, key: &str, size: u64, etag: &str) -> Self {
        Self {
            event_name: "s3:ObjectCreated:Copy".to_string(),
            event_time: Utc::now(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            size,
            etag: etag.to_string(),
            version_id: None,
            source_ip: None,
        }
    }

    /// Check if this event matches a notification filter rule.
    pub fn matches_filter(&self, filter: &Option<NotificationFilter>) -> bool {
        let Some(f) = filter else { return true };
        f.key.filter_rules.iter().all(|rule| match rule.name.as_str() {
            "prefix" => self.key.starts_with(&rule.value),
            "suffix" => self.key.ends_with(&rule.value),
            _ => true,
        })
    }
}
