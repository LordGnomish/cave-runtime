// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;
use std::sync::Arc;
use chrono::{DateTime, Utc};
use tokio::sync::{broadcast, RwLock};
use crate::error::{StoreError, StoreResult};
use crate::types::{
    Bucket, BucketPolicy, CannedAcl, EncryptionInfo, LifecycleRule, MultipartUpload,
    NotificationConfig, ObjectInfo, ObjectVersion, StorageClass, VersioningState,
};
use crate::versioning::{compute_etag, generate_version_id};

#[derive(Clone, Debug)]
pub struct StoreEvent {
    pub event_type: String,
    pub bucket: String,
    pub key: String,
    pub version_id: Option<String>,
}

pub struct ObjectStore {
    pub(crate) buckets: Arc<RwLock<HashMap<String, Bucket>>>,
    pub(crate) uploads: Arc<RwLock<HashMap<String, MultipartUpload>>>,
    pub(crate) notification_sender: broadcast::Sender<StoreEvent>,
}

pub struct BucketInfo {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub region: String,
}

pub struct ListResult {
    pub objects: Vec<ObjectInfo>,
    pub common_prefixes: Vec<String>,
    pub is_truncated: bool,
    pub continuation_token: Option<String>,
    pub next_continuation_token: Option<String>,
}

pub struct ObjectVersionInfo {
    pub key: String,
    pub version_id: String,
    pub is_latest: bool,
    pub is_delete_marker: bool,
    pub last_modified: DateTime<Utc>,
    pub size: u64,
}

fn validate_bucket_name(name: &str) -> StoreResult<()> {
    if name.is_empty() || name.len() > 63 {
        return Err(StoreError::InvalidBucket(name.to_string()));
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.') {
        return Err(StoreError::InvalidBucket(name.to_string()));
    }
    Ok(())
}

impl ObjectStore {
    pub fn new() -> Self {
        let (notification_sender, _) = broadcast::channel(256);
        Self {
            buckets: Arc::new(RwLock::new(HashMap::new())),
            uploads: Arc::new(RwLock::new(HashMap::new())),
            notification_sender,
        }
    }

    // ── Bucket operations ──────────────────────────────────────────────────

    pub async fn create_bucket(&self, name: &str, region: &str) -> StoreResult<()> {
        validate_bucket_name(name)?;
        let mut buckets = self.buckets.write().await;
        if buckets.contains_key(name) {
            return Err(StoreError::BucketExists(name.to_string()));
        }
        buckets.insert(
            name.to_string(),
            Bucket {
                name: name.to_string(),
                created_at: Utc::now(),
                region: region.to_string(),
                versioning: VersioningState::Disabled,
                policy: None,
                acl: CannedAcl::Private,
                lifecycle_rules: vec![],
                notification_config: None,
                objects: HashMap::new(),
            },
        );
        Ok(())
    }

    pub async fn delete_bucket(&self, name: &str) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        if buckets.remove(name).is_none() {
            return Err(StoreError::BucketNotFound(name.to_string()));
        }
        Ok(())
    }

    pub async fn list_buckets(&self) -> Vec<BucketInfo> {
        let buckets = self.buckets.read().await;
        buckets
            .values()
            .map(|b| BucketInfo {
                name: b.name.clone(),
                created_at: b.created_at,
                region: b.region.clone(),
            })
            .collect()
    }

    pub async fn head_bucket(&self, name: &str) -> StoreResult<BucketInfo> {
        let buckets = self.buckets.read().await;
        buckets
            .get(name)
            .map(|b| BucketInfo {
                name: b.name.clone(),
                created_at: b.created_at,
                region: b.region.clone(),
            })
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))
    }

    pub async fn set_bucket_versioning(&self, name: &str, state: VersioningState) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        let bucket = buckets
            .get_mut(name)
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))?;
        bucket.versioning = state;
        Ok(())
    }

    pub async fn get_bucket_versioning(&self, name: &str) -> StoreResult<VersioningState> {
        let buckets = self.buckets.read().await;
        let bucket = buckets
            .get(name)
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))?;
        Ok(bucket.versioning.clone())
    }

    pub async fn put_bucket_policy(&self, name: &str, policy: BucketPolicy) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        let bucket = buckets
            .get_mut(name)
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))?;
        bucket.policy = Some(policy);
        Ok(())
    }

    pub async fn get_bucket_policy(&self, name: &str) -> StoreResult<BucketPolicy> {
        let buckets = self.buckets.read().await;
        let bucket = buckets
            .get(name)
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))?;
        bucket
            .policy
            .clone()
            .ok_or_else(|| StoreError::ObjectNotFound(format!("no policy on bucket {}", name)))
    }

    pub async fn put_bucket_acl(&self, name: &str, acl: CannedAcl) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        let bucket = buckets
            .get_mut(name)
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))?;
        bucket.acl = acl;
        Ok(())
    }

    pub async fn put_lifecycle_rules(&self, name: &str, rules: Vec<LifecycleRule>) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        let bucket = buckets
            .get_mut(name)
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))?;
        bucket.lifecycle_rules = rules;
        Ok(())
    }

    pub async fn put_notification_config(
        &self,
        name: &str,
        config: NotificationConfig,
    ) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        let bucket = buckets
            .get_mut(name)
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))?;
        bucket.notification_config = Some(config);
        Ok(())
    }

    // ── Object operations ──────────────────────────────────────────────────

    pub async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
        metadata: HashMap<String, String>,
        encryption: Option<EncryptionInfo>,
    ) -> StoreResult<ObjectInfo> {
        let etag = compute_etag(&data);
        let size = data.len() as u64;
        let version_id = generate_version_id();
        let now = Utc::now();

        let mut buckets = self.buckets.write().await;
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;

        let versioning_enabled = b.versioning == VersioningState::Enabled;

        let new_version = ObjectVersion {
            version_id: version_id.clone(),
            is_latest: true,
            is_delete_marker: false,
            size,
            etag: etag.clone(),
            content_type: content_type.to_string(),
            metadata: metadata.clone(),
            last_modified: now,
            data,
            encryption,
            storage_class: StorageClass::Standard,
        };

        let versions = b.objects.entry(key.to_string()).or_insert_with(Vec::new);

        if versioning_enabled {
            // Mark existing versions as not latest
            for v in versions.iter_mut() {
                v.is_latest = false;
            }
            versions.insert(0, new_version);
        } else {
            // Replace all versions
            *versions = vec![new_version];
        }

        let _ = self.notification_sender.send(StoreEvent {
            event_type: "s3:ObjectCreated:Put".to_string(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            version_id: Some(version_id.clone()),
        });

        Ok(ObjectInfo {
            bucket: bucket.to_string(),
            key: key.to_string(),
            version_id: if versioning_enabled { Some(version_id) } else { None },
            size,
            etag,
            content_type: content_type.to_string(),
            last_modified: now,
            metadata,
            storage_class: StorageClass::Standard,
        })
    }

    pub async fn get_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> StoreResult<(ObjectVersion, Vec<u8>)> {
        let buckets = self.buckets.read().await;
        let b = buckets
            .get(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
        let versions = b
            .objects
            .get(key)
            .ok_or_else(|| StoreError::ObjectNotFound(key.to_string()))?;

        let version = if let Some(vid) = version_id {
            versions
                .iter()
                .find(|v| v.version_id == vid)
                .ok_or_else(|| StoreError::ObjectNotFound(format!("{}@{}", key, vid)))?
        } else {
            versions
                .iter()
                .find(|v| v.is_latest && !v.is_delete_marker)
                .ok_or_else(|| StoreError::ObjectNotFound(key.to_string()))?
        };

        Ok((version.clone(), version.data.clone()))
    }

    pub async fn delete_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;

        if let Some(vid) = version_id {
            let versions = b
                .objects
                .get_mut(key)
                .ok_or_else(|| StoreError::ObjectNotFound(key.to_string()))?;
            let before = versions.len();
            versions.retain(|v| v.version_id != vid);
            if versions.len() == before {
                return Err(StoreError::ObjectNotFound(format!("{}@{}", key, vid)));
            }
            if versions.is_empty() {
                b.objects.remove(key);
            } else if !versions.iter().any(|v| v.is_latest) {
                if let Some(v) = versions.first_mut() {
                    v.is_latest = true;
                }
            }
        } else if b.versioning == VersioningState::Enabled {
            // Insert delete marker
            let versions = b.objects.entry(key.to_string()).or_insert_with(Vec::new);
            for v in versions.iter_mut() {
                v.is_latest = false;
            }
            versions.insert(
                0,
                ObjectVersion {
                    version_id: generate_version_id(),
                    is_latest: true,
                    is_delete_marker: true,
                    size: 0,
                    etag: String::new(),
                    content_type: String::new(),
                    metadata: HashMap::new(),
                    last_modified: Utc::now(),
                    data: vec![],
                    encryption: None,
                    storage_class: StorageClass::Standard,
                },
            );
        } else {
            b.objects.remove(key);
        }

        let _ = self.notification_sender.send(StoreEvent {
            event_type: "s3:ObjectRemoved:Delete".to_string(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            version_id: None,
        });

        Ok(())
    }

    pub async fn head_object(&self, bucket: &str, key: &str) -> StoreResult<ObjectInfo> {
        let buckets = self.buckets.read().await;
        let b = buckets
            .get(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
        let versions = b
            .objects
            .get(key)
            .ok_or_else(|| StoreError::ObjectNotFound(key.to_string()))?;
        let version = versions
            .iter()
            .find(|v| v.is_latest && !v.is_delete_marker)
            .ok_or_else(|| StoreError::ObjectNotFound(key.to_string()))?;

        Ok(ObjectInfo {
            bucket: bucket.to_string(),
            key: key.to_string(),
            version_id: Some(version.version_id.clone()),
            size: version.size,
            etag: version.etag.clone(),
            content_type: version.content_type.clone(),
            last_modified: version.last_modified,
            metadata: version.metadata.clone(),
            storage_class: version.storage_class.clone(),
        })
    }

    pub async fn copy_object(
        &self,
        src_bucket: &str,
        src_key: &str,
        dst_bucket: &str,
        dst_key: &str,
    ) -> StoreResult<ObjectInfo> {
        // Read source
        let (src_version, data) = self.get_object(src_bucket, src_key, None).await?;
        // Write to destination
        self.put_object(
            dst_bucket,
            dst_key,
            data,
            &src_version.content_type,
            src_version.metadata.clone(),
            src_version.encryption.clone(),
        )
        .await
    }

    pub async fn list_objects_v2(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        delimiter: Option<&str>,
        max_keys: Option<usize>,
        continuation_token: Option<&str>,
    ) -> StoreResult<ListResult> {
        let buckets = self.buckets.read().await;
        let b = buckets
            .get(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;

        let prefix = prefix.unwrap_or("");
        let max_keys = max_keys.unwrap_or(1000);

        let mut all_keys: Vec<&String> = b
            .objects
            .keys()
            .filter(|k| k.starts_with(prefix))
            .collect();
        all_keys.sort();

        // Handle continuation token (it's a key to start after)
        let start_idx = if let Some(token) = continuation_token {
            all_keys
                .iter()
                .position(|k| k.as_str() > token)
                .unwrap_or(all_keys.len())
        } else {
            0
        };

        let mut objects = Vec::new();
        let mut common_prefixes: std::collections::HashSet<String> = std::collections::HashSet::new();

        for key in all_keys.iter().skip(start_idx).take(max_keys + 1) {
            if objects.len() >= max_keys {
                break;
            }

            // Check delimiter for common prefixes
            if let Some(delim) = delimiter {
                let suffix = &key[prefix.len()..];
                if let Some(delim_pos) = suffix.find(delim) {
                    let cp = format!("{}{}{}", prefix, &suffix[..delim_pos], delim);
                    common_prefixes.insert(cp);
                    continue;
                }
            }

            if let Some(versions) = b.objects.get(*key) {
                if let Some(v) = versions.iter().find(|v| v.is_latest && !v.is_delete_marker) {
                    objects.push(ObjectInfo {
                        bucket: bucket.to_string(),
                        key: key.to_string(),
                        version_id: Some(v.version_id.clone()),
                        size: v.size,
                        etag: v.etag.clone(),
                        content_type: v.content_type.clone(),
                        last_modified: v.last_modified,
                        metadata: v.metadata.clone(),
                        storage_class: v.storage_class.clone(),
                    });
                }
            }
        }

        let is_truncated = objects.len() >= max_keys;
        let next_token = if is_truncated {
            objects.last().map(|o| o.key.clone())
        } else {
            None
        };

        let mut cp_list: Vec<String> = common_prefixes.into_iter().collect();
        cp_list.sort();

        Ok(ListResult {
            objects,
            common_prefixes: cp_list,
            is_truncated,
            continuation_token: continuation_token.map(|s| s.to_string()),
            next_continuation_token: next_token,
        })
    }

    pub async fn list_object_versions(
        &self,
        bucket: &str,
        prefix: Option<&str>,
    ) -> StoreResult<Vec<ObjectVersionInfo>> {
        let buckets = self.buckets.read().await;
        let b = buckets
            .get(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;

        let prefix = prefix.unwrap_or("");
        let mut result = Vec::new();

        for (key, versions) in &b.objects {
            if !key.starts_with(prefix) {
                continue;
            }
            for v in versions {
                result.push(ObjectVersionInfo {
                    key: key.clone(),
                    version_id: v.version_id.clone(),
                    is_latest: v.is_latest,
                    is_delete_marker: v.is_delete_marker,
                    last_modified: v.last_modified,
                    size: v.size,
                });
            }
        }

        Ok(result)
    }
}

impl Default for ObjectStore {
    fn default() -> Self {
        Self::new()
    }
}
