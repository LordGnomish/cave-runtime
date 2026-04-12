//! S3-compatible data types and XML serialisation helpers.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Bucket types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketInfo {
    pub name: String,
    pub creation_date: DateTime<Utc>,
    pub region: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VersioningStatus {
    Off,
    Enabled,
    Suspended,
}

impl Default for VersioningStatus {
    fn default() -> Self { Self::Off }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BucketVersioning {
    pub status: VersioningStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleRule {
    pub id: String,
    pub prefix: String,
    pub expiration_days: Option<u32>,
    pub noncurrent_version_expiration_days: Option<u32>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BucketPolicy {
    pub version: String,
    pub statements: Vec<PolicyStatement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyStatement {
    pub sid: String,
    pub effect: String,
    pub principal: serde_json::Value,
    pub action: Vec<String>,
    pub resource: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationConfig {
    pub queue_configs: Vec<QueueConfig>,
    pub topic_configs: Vec<TopicConfig>,
    pub lambda_configs: Vec<LambdaConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    pub id: String,
    pub queue_arn: String,
    pub events: Vec<String>,
    pub prefix_filter: Option<String>,
    pub suffix_filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicConfig {
    pub id: String,
    pub topic_arn: String,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LambdaConfig {
    pub id: String,
    pub lambda_arn: String,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketAcl {
    pub owner_id: String,
    pub owner_display_name: String,
    pub grants: Vec<Grant>,
}

impl Default for BucketAcl {
    fn default() -> Self {
        Self {
            owner_id: "cave-store-owner".to_string(),
            owner_display_name: "cave-store".to_string(),
            grants: vec![Grant {
                grantee_type: "CanonicalUser".to_string(),
                grantee_id: "cave-store-owner".to_string(),
                permission: "FULL_CONTROL".to_string(),
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grant {
    pub grantee_type: String,
    pub grantee_id: String,
    pub permission: String,
}

// ─── Object types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMetadata {
    pub key: String,
    pub size: u64,
    pub etag: String,
    pub last_modified: DateTime<Utc>,
    pub content_type: String,
    pub storage_class: String,
    pub version_id: Option<String>,
    pub user_metadata: std::collections::HashMap<String, String>,
    pub sse_algorithm: Option<SseAlgorithm>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SseAlgorithm {
    /// Server-side encryption with S3-managed keys
    AwsS3,
    /// Server-side encryption with customer-provided keys
    AwsKms,
    /// Customer-provided encryption key (SSE-C)
    Customer,
}

#[derive(Debug, Clone)]
pub struct SseConfig {
    pub algorithm: SseAlgorithm,
    /// For SSE-C: the raw key bytes (32 for AES-256)
    pub customer_key: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectVersion {
    pub version_id: String,
    pub is_delete_marker: bool,
    pub metadata: ObjectMetadata,
}

// ─── Multipart upload ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedPart {
    pub part_number: u32,
    pub etag: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultipartUpload {
    pub upload_id: String,
    pub bucket: String,
    pub key: String,
    pub initiated: DateTime<Utc>,
    pub parts: BTreeMap<u32, UploadedPart>,
    pub metadata: std::collections::HashMap<String, String>,
    pub content_type: String,
}

// ─── List results ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ListObjectsV2Result {
    pub bucket: String,
    pub prefix: String,
    pub delimiter: String,
    pub max_keys: u32,
    pub key_count: u32,
    pub truncated: bool,
    pub next_continuation_token: Option<String>,
    pub contents: Vec<ObjectMetadata>,
    pub common_prefixes: Vec<String>,
}

// ─── XML helpers ──────────────────────────────────────────────────────────────

/// Format a DateTime as S3's preferred ISO-8601 format.
pub fn fmt_time(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S.000Z").to_string()
}

/// Build a minimal S3 error XML body.
pub fn error_xml(code: &str, message: &str, resource: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Error>
  <Code>{code}</Code>
  <Message>{message}</Message>
  <Resource>{resource}</Resource>
  <RequestId>cave-store</RequestId>
</Error>"#
    )
}

/// ListBuckets XML response body.
pub fn list_buckets_xml(buckets: &[BucketInfo]) -> String {
    let mut body = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListAllMyBucketsResult>
  <Owner><ID>cave-store-owner</ID><DisplayName>cave-store</DisplayName></Owner>
  <Buckets>"#,
    );
    for b in buckets {
        body.push_str(&format!(
            "\n    <Bucket><Name>{}</Name><CreationDate>{}</CreationDate></Bucket>",
            b.name,
            fmt_time(&b.creation_date)
        ));
    }
    body.push_str("\n  </Buckets>\n</ListAllMyBucketsResult>");
    body
}

/// ListObjectsV2 XML response body.
pub fn list_objects_v2_xml(result: &ListObjectsV2Result) -> String {
    let mut body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult>
  <Name>{}</Name>
  <Prefix>{}</Prefix>
  <MaxKeys>{}</MaxKeys>
  <KeyCount>{}</KeyCount>
  <IsTruncated>{}</IsTruncated>"#,
        result.bucket, result.prefix, result.max_keys, result.key_count, result.truncated
    );
    if let Some(ref token) = result.next_continuation_token {
        body.push_str(&format!("\n  <NextContinuationToken>{token}</NextContinuationToken>"));
    }
    for obj in &result.contents {
        body.push_str(&format!(
            r#"
  <Contents>
    <Key>{}</Key>
    <LastModified>{}</LastModified>
    <ETag>&quot;{}&quot;</ETag>
    <Size>{}</Size>
    <StorageClass>{}</StorageClass>
  </Contents>"#,
            obj.key,
            fmt_time(&obj.last_modified),
            obj.etag,
            obj.size,
            obj.storage_class
        ));
    }
    for prefix in &result.common_prefixes {
        body.push_str(&format!("\n  <CommonPrefixes><Prefix>{prefix}</Prefix></CommonPrefixes>"));
    }
    body.push_str("\n</ListBucketResult>");
    body
}

/// CompleteMultipartUpload XML response body.
pub fn complete_multipart_xml(bucket: &str, key: &str, etag: &str, location: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult>
  <Location>{location}</Location>
  <Bucket>{bucket}</Bucket>
  <Key>{key}</Key>
  <ETag>&quot;{etag}&quot;</ETag>
</CompleteMultipartUploadResult>"#
    )
}

/// InitiateMultipartUpload XML response.
pub fn initiate_multipart_xml(bucket: &str, key: &str, upload_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult>
  <Bucket>{bucket}</Bucket>
  <Key>{key}</Key>
  <UploadId>{upload_id}</UploadId>
</InitiateMultipartUploadResult>"#
    )
}

/// AccessControlPolicy XML response.
pub fn acl_xml(acl: &BucketAcl) -> String {
    let mut body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<AccessControlPolicy>
  <Owner><ID>{}</ID><DisplayName>{}</DisplayName></Owner>
  <AccessControlList>"#,
        acl.owner_id, acl.owner_display_name
    );
    for grant in &acl.grants {
        body.push_str(&format!(
            r#"
    <Grant>
      <Grantee xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="{}">
        <ID>{}</ID>
      </Grantee>
      <Permission>{}</Permission>
    </Grant>"#,
            grant.grantee_type, grant.grantee_id, grant.permission
        ));
    }
    body.push_str("\n  </AccessControlList>\n</AccessControlPolicy>");
    body
}

pub fn versioning_xml(v: &BucketVersioning) -> String {
    let status = match v.status {
        VersioningStatus::Off => "",
        VersioningStatus::Enabled => "Enabled",
        VersioningStatus::Suspended => "Suspended",
    };
    if status.is_empty() {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<VersioningConfiguration/>"#
            .to_string()
    } else {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<VersioningConfiguration>
  <Status>{status}</Status>
</VersioningConfiguration>"#
        )
    }
}

pub fn list_multipart_uploads_xml(bucket: &str, uploads: &[MultipartUpload]) -> String {
    let mut body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ListMultipartUploadsResult>
  <Bucket>{bucket}</Bucket>
  <MaxUploads>1000</MaxUploads>
  <IsTruncated>false</IsTruncated>"#
    );
    for u in uploads {
        body.push_str(&format!(
            r#"
  <Upload>
    <Key>{}</Key>
    <UploadId>{}</UploadId>
    <Initiated>{}</Initiated>
  </Upload>"#,
            u.key, u.upload_id, fmt_time(&u.initiated)
        ));
    }
    body.push_str("\n</ListMultipartUploadsResult>");
    body
}
