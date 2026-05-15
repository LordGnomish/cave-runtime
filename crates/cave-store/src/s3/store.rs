// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core S3/MinIO object store implementation.
//!
//! Objects are stored as files on disk (data_dir/bucket/object-path).
//! Metadata and bucket configuration live in memory, backed by WAL.

use crate::error::{StoreError, StoreResult};
use crate::s3::encryption::{self, ServerKeyStore};
use crate::s3::lifecycle;
use crate::s3::notification;
use crate::s3::policy::{evaluate, BucketPolicy, Effect, PolicyContext};
use crate::s3::types::*;
use crate::wal::{WalEntry, WalWriter};
use base64::Engine;
use chrono::Utc;
use ring::digest;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct ObjectStore {
    data_dir: PathBuf,
    buckets: RwLock<HashMap<String, Bucket>>,
    /// (bucket, key) → ordered list of versions (oldest first)
    objects: RwLock<HashMap<(String, String), Vec<ObjectVersion>>>,
    multiparts: RwLock<HashMap<String, MultipartUpload>>,
    pub event_tx: broadcast::Sender<S3Event>,
    wal: Arc<WalWriter>,
    key_store: RwLock<ServerKeyStore>,
}

impl ObjectStore {
    pub fn new(data_dir: PathBuf, wal: Arc<WalWriter>) -> Self {
        let (tx, _) = broadcast::channel(4096);
        Self {
            data_dir,
            buckets: RwLock::new(HashMap::new()),
            objects: RwLock::new(HashMap::new()),
            multiparts: RwLock::new(HashMap::new()),
            event_tx: tx,
            wal,
            key_store: RwLock::new(ServerKeyStore::new()),
        }
    }

    /// Replay S3-related WAL entries on startup.
    pub async fn replay_wal(&self, entries: &[WalEntry]) {
        for entry in entries {
            match entry {
                WalEntry::BucketCreate { name, region, owner } => {
                    self.buckets.write().await.insert(
                        name.clone(),
                        Bucket::new(name.clone(), region.clone(), owner.clone()),
                    );
                }
                WalEntry::BucketDelete { name } => {
                    self.buckets.write().await.remove(name);
                }
                WalEntry::BucketVersioning { name, state } => {
                    if let Some(b) = self.buckets.write().await.get_mut(name) {
                        b.versioning = match state.as_str() {
                            "Enabled" => VersioningState::Enabled,
                            "Suspended" => VersioningState::Suspended,
                            _ => VersioningState::Disabled,
                        };
                    }
                }
                WalEntry::BucketPolicy { name, policy_json } => {
                    if let Some(b) = self.buckets.write().await.get_mut(name) {
                        b.policy = Some(policy_json.clone());
                    }
                }
                WalEntry::BucketLifecycle { name, rules_json } => {
                    if let Some(b) = self.buckets.write().await.get_mut(name) {
                        if let Ok(rules) = serde_json::from_str(rules_json) {
                            b.lifecycle_rules = rules;
                        }
                    }
                }
                WalEntry::BucketNotification { name, config_json } => {
                    if let Some(b) = self.buckets.write().await.get_mut(name) {
                        if let Ok(cfg) = serde_json::from_str(config_json) {
                            b.notification_config = cfg;
                        }
                    }
                }
                WalEntry::ObjectPut {
                    bucket,
                    key,
                    version_id,
                    etag,
                    size,
                    content_type,
                    metadata_json,
                    storage_path,
                    ..
                } => {
                    let metadata = serde_json::from_str(metadata_json).unwrap_or_default();
                    let v = ObjectVersion {
                        version_id: version_id.clone(),
                        etag: etag.clone(),
                        size: *size,
                        last_modified: Utc::now(),
                        content_type: content_type.clone(),
                        metadata,
                        tags: HashMap::new(),
                        storage_class: StorageClass::Standard,
                        storage_path: storage_path.clone(),
                        encryption: None,
                        delete_marker: false,
                        restore_status: None,
                    };
                    self.objects
                        .write()
                        .await
                        .entry((bucket.clone(), key.clone()))
                        .or_default()
                        .push(v);
                }
                WalEntry::ObjectDelete {
                    bucket,
                    key,
                    version_id,
                    delete_marker,
                } => {
                    let mut objects = self.objects.write().await;
                    if *delete_marker {
                        let dm = ObjectVersion {
                            version_id: version_id.clone(),
                            etag: String::new(),
                            size: 0,
                            last_modified: Utc::now(),
                            content_type: String::new(),
                            metadata: HashMap::new(),
                            tags: HashMap::new(),
                            storage_class: StorageClass::Standard,
                            storage_path: String::new(),
                            encryption: None,
                            delete_marker: true,
                            restore_status: None,
                        };
                        objects
                            .entry((bucket.clone(), key.clone()))
                            .or_default()
                            .push(dm);
                    } else if let Some(vid) = version_id {
                        if let Some(versions) = objects.get_mut(&(bucket.clone(), key.clone())) {
                            versions.retain(|v| v.version_id.as_deref() != Some(vid.as_str()));
                        }
                    } else {
                        objects.remove(&(bucket.clone(), key.clone()));
                    }
                }
                WalEntry::MultipartInit {
                    upload_id,
                    bucket,
                    key,
                    metadata_json,
                } => {
                    let metadata = serde_json::from_str(metadata_json).unwrap_or_default();
                    self.multiparts.write().await.insert(
                        upload_id.clone(),
                        MultipartUpload {
                            upload_id: upload_id.clone(),
                            bucket: bucket.clone(),
                            key: key.clone(),
                            initiated: Utc::now(),
                            owner: "cave".to_string(),
                            content_type: String::new(),
                            metadata,
                            parts: HashMap::new(),
                        },
                    );
                }
                WalEntry::MultipartPart {
                    upload_id,
                    part_number,
                    etag,
                    size,
                    storage_path,
                } => {
                    if let Some(mp) = self.multiparts.write().await.get_mut(upload_id) {
                        mp.parts.insert(
                            *part_number,
                            UploadedPart {
                                part_number: *part_number,
                                etag: etag.clone(),
                                size: *size,
                                storage_path: storage_path.clone(),
                                last_modified: Utc::now(),
                            },
                        );
                    }
                }
                WalEntry::MultipartComplete { upload_id, .. } => {
                    self.multiparts.write().await.remove(upload_id);
                }
                WalEntry::MultipartAbort { upload_id } => {
                    self.multiparts.write().await.remove(upload_id);
                }
                _ => {}
            }
        }
        info!("S3 store WAL replay complete");
    }

    // ── Bucket operations ───────────────────────────────────────────────────────

    pub async fn create_bucket(
        &self,
        name: &str,
        region: &str,
        owner: &str,
    ) -> StoreResult<()> {
        validate_bucket_name(name)?;
        let mut buckets = self.buckets.write().await;
        if buckets.contains_key(name) {
            return Err(StoreError::BucketAlreadyExists(name.to_string()));
        }
        std::fs::create_dir_all(self.data_dir.join(name))?;
        buckets.insert(name.to_string(), Bucket::new(name.to_string(), region.to_string(), owner.to_string()));
        drop(buckets);
        self.wal
            .append(&WalEntry::BucketCreate {
                name: name.to_string(),
                region: region.to_string(),
                owner: owner.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn delete_bucket(&self, name: &str) -> StoreResult<()> {
        let objects = self.objects.read().await;
        let has_objects = objects.keys().any(|(b, _)| b == name);
        if has_objects {
            return Err(StoreError::BucketNotEmpty(name.to_string()));
        }
        drop(objects);
        self.buckets
            .write()
            .await
            .remove(name)
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))?;
        let _ = std::fs::remove_dir_all(self.data_dir.join(name));
        self.wal
            .append(&WalEntry::BucketDelete { name: name.to_string() })
            .await?;
        Ok(())
    }

    pub async fn list_buckets(&self) -> Vec<Bucket> {
        let mut buckets: Vec<Bucket> = self.buckets.read().await.values().cloned().collect();
        buckets.sort_by(|a, b| a.name.cmp(&b.name));
        buckets
    }

    pub async fn head_bucket(&self, name: &str) -> StoreResult<()> {
        self.buckets
            .read()
            .await
            .get(name)
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))?;
        Ok(())
    }

    pub async fn get_bucket(&self, name: &str) -> StoreResult<Bucket> {
        self.buckets
            .read()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))
    }

    pub async fn set_versioning(&self, bucket: &str, state: VersioningState) -> StoreResult<()> {
        let state_str = match &state {
            VersioningState::Enabled => "Enabled",
            VersioningState::Suspended => "Suspended",
            VersioningState::Disabled => "Disabled",
        }
        .to_string();
        let mut buckets = self.buckets.write().await;
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
        b.versioning = state;
        drop(buckets);
        self.wal
            .append(&WalEntry::BucketVersioning {
                name: bucket.to_string(),
                state: state_str,
            })
            .await?;
        Ok(())
    }

    pub async fn put_bucket_policy(&self, bucket: &str, policy_json: &str) -> StoreResult<()> {
        // Validate JSON is parseable as a policy
        let _: BucketPolicy = serde_json::from_str(policy_json)
            .map_err(|e| StoreError::InvalidArgument(format!("invalid policy: {e}")))?;
        let mut buckets = self.buckets.write().await;
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
        b.policy = Some(policy_json.to_string());
        drop(buckets);
        self.wal
            .append(&WalEntry::BucketPolicy {
                name: bucket.to_string(),
                policy_json: policy_json.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn delete_bucket_policy(&self, bucket: &str) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
        b.policy = None;
        Ok(())
    }

    pub async fn put_bucket_lifecycle(
        &self,
        bucket: &str,
        rules: Vec<LifecycleRule>,
    ) -> StoreResult<()> {
        let rules_json = serde_json::to_string(&rules)?;
        let mut buckets = self.buckets.write().await;
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
        b.lifecycle_rules = rules;
        drop(buckets);
        self.wal
            .append(&WalEntry::BucketLifecycle {
                name: bucket.to_string(),
                rules_json,
            })
            .await?;
        Ok(())
    }

    pub async fn put_bucket_notification(
        &self,
        bucket: &str,
        config: NotificationConfiguration,
    ) -> StoreResult<()> {
        let config_json = serde_json::to_string(&config)?;
        let mut buckets = self.buckets.write().await;
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
        b.notification_config = config;
        drop(buckets);
        self.wal
            .append(&WalEntry::BucketNotification {
                name: bucket.to_string(),
                config_json,
            })
            .await?;
        Ok(())
    }

    pub async fn put_bucket_encryption(
        &self,
        bucket: &str,
        enc: BucketEncryption,
    ) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
        b.encryption = Some(enc);
        Ok(())
    }

    pub async fn put_bucket_tags(
        &self,
        bucket: &str,
        tags: HashMap<String, String>,
    ) -> StoreResult<()> {
        let mut buckets = self.buckets.write().await;
        let b = buckets
            .get_mut(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
        b.tags = tags;
        Ok(())
    }

    // ── Object operations ───────────────────────────────────────────────────────

    pub async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
        metadata: HashMap<String, String>,
        tags: HashMap<String, String>,
        sse: Option<&str>,          // "AES256" | "aws:kms" | None
        sse_key_b64: Option<&str>,  // SSE-C key
        storage_class: Option<StorageClass>,
    ) -> StoreResult<PutObjectResult> {
        // Ensure bucket exists
        let (versioning, bucket_enc, notification_cfg) = {
            let buckets = self.buckets.read().await;
            let b = buckets
                .get(bucket)
                .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
            (b.versioning.clone(), b.encryption.clone(), b.notification_config.clone())
        };

        // Determine version ID
        let version_id = match versioning {
            VersioningState::Enabled => Some(Uuid::new_v4().to_string()),
            _ => None,
        };

        // Compute etag (MD5 equivalent via SHA256 prefix)
        let etag = compute_etag(&data);
        let size = data.len() as u64;

        // Encrypt if requested
        let (stored_data, enc_meta) = match sse.or(bucket_enc.as_ref().map(|e| match e.sse_algorithm {
            SseAlgorithm::Aes256 => "AES256",
            SseAlgorithm::AwsKms => "aws:kms",
        })) {
            Some("AES256") => {
                if let Some(sse_key_b64) = sse_key_b64 {
                    // SSE-C
                    let key = encryption::parse_sse_c_key(sse_key_b64)?;
                    let ciphertext = encryption::encrypt_aes256gcm(&data, &key)?;
                    let md5 = encryption::key_md5(&key);
                    (ciphertext, Some(ObjectEncryption {
                        algorithm: "SSE-C".to_string(),
                        key_md5: Some(md5),
                        kms_key_id: None,
                    }))
                } else {
                    // SSE-S3
                    let ks = self.key_store.read().await;
                    let (key_id, root_key) = ks.current_key();
                    let obj_key = encryption::derive_sse_s3_key(root_key, &format!("{bucket}/{key}"));
                    let ciphertext = encryption::encrypt_aes256gcm(&data, &obj_key)?;
                    (ciphertext, Some(ObjectEncryption {
                        algorithm: "AES256".to_string(),
                        key_md5: Some(key_id.to_string()),
                        kms_key_id: None,
                    }))
                }
            }
            _ => (data, None),
        };

        // Write to disk
        let rel_path = object_path(bucket, key, version_id.as_deref());
        let abs_path = self.data_dir.join(&rel_path);
        if let Some(parent) = abs_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&abs_path, &stored_data)?;

        let version = ObjectVersion {
            version_id: version_id.clone(),
            etag: etag.clone(),
            size,
            last_modified: Utc::now(),
            content_type: content_type.to_string(),
            metadata: metadata.clone(),
            tags,
            storage_class: storage_class.unwrap_or(StorageClass::Standard),
            storage_path: rel_path.to_string_lossy().to_string(),
            encryption: enc_meta,
            delete_marker: false,
            restore_status: None,
        };

        let meta_json = serde_json::to_string(&metadata)?;
        self.objects
            .write()
            .await
            .entry((bucket.to_string(), key.to_string()))
            .or_default()
            .push(version);

        self.wal
            .append(&WalEntry::ObjectPut {
                bucket: bucket.to_string(),
                key: key.to_string(),
                version_id: version_id.clone(),
                etag: etag.clone(),
                size,
                content_type: content_type.to_string(),
                metadata_json: meta_json,
                storage_path: rel_path.to_string_lossy().to_string(),
                lease_id: None,
            })
            .await?;

        // Dispatch event
        let event = S3Event::object_created_put(bucket, key, size, &etag);
        notification::dispatch(&notification_cfg, &event, &self.event_tx).await;

        Ok(PutObjectResult { etag, version_id })
    }

    pub async fn get_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
        range: Option<(u64, u64)>,
        sse_c_key: Option<&str>,
    ) -> StoreResult<GetObjectResult> {
        let objects = self.objects.read().await;
        let versions = objects
            .get(&(bucket.to_string(), key.to_string()))
            .ok_or_else(|| StoreError::ObjectNotFound {
                bucket: bucket.to_string(),
                key: key.to_string(),
            })?;

        let version = if let Some(vid) = version_id {
            versions
                .iter()
                .rev()
                .find(|v| v.version_id.as_deref() == Some(vid))
        } else {
            versions.last()
        }
        .ok_or_else(|| StoreError::ObjectNotFound {
            bucket: bucket.to_string(),
            key: key.to_string(),
        })?;

        if version.delete_marker {
            return Err(StoreError::ObjectNotFound {
                bucket: bucket.to_string(),
                key: key.to_string(),
            });
        }

        let abs_path = self.data_dir.join(&version.storage_path);
        let raw = std::fs::read(&abs_path)?;

        // Decrypt if encrypted
        let data = match &version.encryption {
            Some(enc) if enc.algorithm == "SSE-C" => {
                let key_b64 = sse_c_key.ok_or_else(|| {
                    StoreError::EncryptionError("SSE-C key required".into())
                })?;
                let key = encryption::parse_sse_c_key(key_b64)?;
                encryption::decrypt_aes256gcm(&raw, &key)?
            }
            Some(enc) if enc.algorithm == "AES256" => {
                // SSE-S3: re-derive key
                let ks = self.key_store.read().await;
                let key_id = enc.key_md5.as_deref().unwrap_or("");
                let root_key = ks
                    .get_key(key_id)
                    .ok_or_else(|| StoreError::EncryptionError("encryption key not found".into()))?;
                let obj_key = encryption::derive_sse_s3_key(root_key, &format!("{bucket}/{key}"));
                encryption::decrypt_aes256gcm(&raw, &obj_key)?
            }
            _ => raw,
        };

        // Apply range
        let (body, content_range) = if let Some((start, end)) = range {
            let end = end.min(data.len() as u64 - 1);
            let slice = data[start as usize..=end as usize].to_vec();
            let cr = format!("bytes {start}-{end}/{}", data.len());
            (slice, Some(cr))
        } else {
            (data, None)
        };

        Ok(GetObjectResult {
            body,
            content_type: version.content_type.clone(),
            etag: version.etag.clone(),
            last_modified: version.last_modified,
            size: version.size,
            version_id: version.version_id.clone(),
            metadata: version.metadata.clone(),
            storage_class: format!("{:?}", version.storage_class).to_uppercase(),
            content_range,
            encryption: version.encryption.clone(),
            delete_marker: version.delete_marker,
        })
    }

    pub async fn head_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> StoreResult<HeadObjectResult> {
        let objects = self.objects.read().await;
        let versions = objects
            .get(&(bucket.to_string(), key.to_string()))
            .ok_or_else(|| StoreError::ObjectNotFound {
                bucket: bucket.to_string(),
                key: key.to_string(),
            })?;
        let version = if let Some(vid) = version_id {
            versions.iter().rev().find(|v| v.version_id.as_deref() == Some(vid))
        } else {
            versions.last()
        }
        .ok_or_else(|| StoreError::ObjectNotFound {
            bucket: bucket.to_string(),
            key: key.to_string(),
        })?;

        Ok(HeadObjectResult {
            content_type: version.content_type.clone(),
            etag: version.etag.clone(),
            last_modified: version.last_modified,
            size: version.size,
            version_id: version.version_id.clone(),
            metadata: version.metadata.clone(),
            storage_class: format!("{:?}", version.storage_class).to_uppercase(),
            delete_marker: version.delete_marker,
            encryption: version.encryption.clone(),
        })
    }

    pub async fn delete_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> StoreResult<DeleteObjectResult> {
        let (versioning, notification_cfg) = {
            let buckets = self.buckets.read().await;
            let b = buckets
                .get(bucket)
                .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;
            (b.versioning.clone(), b.notification_config.clone())
        };

        let mut objects = self.objects.write().await;
        let versions = objects
            .get_mut(&(bucket.to_string(), key.to_string()))
            .ok_or_else(|| StoreError::ObjectNotFound {
                bucket: bucket.to_string(),
                key: key.to_string(),
            })?;

        let result = if let Some(vid) = version_id {
            // Delete specific version
            let idx = versions.iter().rposition(|v| v.version_id.as_deref() == Some(vid));
            if let Some(i) = idx {
                let removed = versions.remove(i);
                if let Ok(p) = self.data_dir.join(&removed.storage_path).canonicalize() {
                    let _ = std::fs::remove_file(p);
                }
                DeleteObjectResult {
                    version_id: removed.version_id,
                    delete_marker: false,
                }
            } else {
                return Err(StoreError::ObjectNotFound {
                    bucket: bucket.to_string(),
                    key: key.to_string(),
                });
            }
        } else if versioning == VersioningState::Enabled {
            // Insert delete marker
            let new_vid = Uuid::new_v4().to_string();
            versions.push(ObjectVersion {
                version_id: Some(new_vid.clone()),
                etag: String::new(),
                size: 0,
                last_modified: Utc::now(),
                content_type: String::new(),
                metadata: HashMap::new(),
                tags: HashMap::new(),
                storage_class: StorageClass::Standard,
                storage_path: String::new(),
                encryption: None,
                delete_marker: true,
                restore_status: None,
            });
            DeleteObjectResult {
                version_id: Some(new_vid),
                delete_marker: true,
            }
        } else {
            // Hard delete all versions
            let removed = versions.drain(..).collect::<Vec<_>>();
            for v in &removed {
                if !v.storage_path.is_empty() {
                    let _ = std::fs::remove_file(self.data_dir.join(&v.storage_path));
                }
            }
            objects.remove(&(bucket.to_string(), key.to_string()));
            DeleteObjectResult {
                version_id: None,
                delete_marker: false,
            }
        };

        drop(objects);
        self.wal
            .append(&WalEntry::ObjectDelete {
                bucket: bucket.to_string(),
                key: key.to_string(),
                version_id: result.version_id.clone(),
                delete_marker: result.delete_marker,
            })
            .await?;

        let event = S3Event::object_removed_delete(bucket, key);
        notification::dispatch(&notification_cfg, &event, &self.event_tx).await;

        Ok(result)
    }

    pub async fn copy_object(
        &self,
        src_bucket: &str,
        src_key: &str,
        src_version: Option<&str>,
        dst_bucket: &str,
        dst_key: &str,
        metadata_directive: &str, // "COPY" | "REPLACE"
        new_metadata: Option<HashMap<String, String>>,
    ) -> StoreResult<CopyObjectResult> {
        let src = self.get_object(src_bucket, src_key, src_version, None, None).await?;
        let metadata = match metadata_directive {
            "REPLACE" => new_metadata.unwrap_or_default(),
            _ => src.metadata.clone(),
        };
        let r = self
            .put_object(
                dst_bucket,
                dst_key,
                src.body,
                &src.content_type,
                metadata,
                HashMap::new(),
                None,
                None,
                None,
            )
            .await?;

        let event = S3Event::object_created_copy(dst_bucket, dst_key, src.size, &r.etag);
        let notification_cfg = self.buckets.read().await.get(dst_bucket).map(|b| b.notification_config.clone()).unwrap_or_default();
        notification::dispatch(&notification_cfg, &event, &self.event_tx).await;

        Ok(CopyObjectResult {
            etag: r.etag,
            last_modified: Utc::now(),
            version_id: r.version_id,
        })
    }

    pub async fn list_objects_v2(
        &self,
        bucket: &str,
        prefix: &str,
        delimiter: Option<&str>,
        max_keys: u32,
        continuation_token: Option<&str>,
    ) -> StoreResult<ListResult> {
        self.buckets
            .read()
            .await
            .get(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;

        let objects = self.objects.read().await;
        let mut contents: Vec<ObjectListEntry> = Vec::new();
        let mut common_prefixes: std::collections::BTreeSet<String> = Default::default();
        let mut count = 0u32;
        let mut is_truncated = false;

        let skip_to = continuation_token.unwrap_or("");
        let mut skipping = !skip_to.is_empty();

        let mut keys: Vec<&(String, String)> = objects
            .keys()
            .filter(|(b, k)| b == bucket && k.starts_with(prefix))
            .collect();
        keys.sort_by(|a, b| a.1.cmp(&b.1));

        for (_, key) in keys {
            if skipping {
                if key.as_str() == skip_to {
                    skipping = false;
                }
                continue;
            }
            if count >= max_keys {
                is_truncated = true;
                break;
            }
            // Latest non-delete-marker version
            let versions = &objects[&(bucket.to_string(), key.clone())];
            let latest = versions.iter().rev().find(|v| !v.delete_marker);
            let Some(v) = latest else { continue };

            // Delimiter grouping
            if let Some(delim) = delimiter {
                let suffix = &key[prefix.len()..];
                if let Some(idx) = suffix.find(delim) {
                    let cp = format!("{}{}{}", prefix, &suffix[..=idx], delim);
                    common_prefixes.insert(cp);
                    continue;
                }
            }

            contents.push(ObjectListEntry {
                key: key.clone(),
                last_modified: v.last_modified,
                etag: v.etag.clone(),
                size: v.size,
                storage_class: format!("{:?}", v.storage_class).to_uppercase(),
            });
            count += 1;
        }

        Ok(ListResult {
            contents,
            common_prefixes: common_prefixes.into_iter().collect(),
            is_truncated,
            key_count: count,
            next_continuation_token: if is_truncated {
                contents_last_key(&*objects, bucket, prefix, max_keys, continuation_token)
            } else {
                None
            },
        })
    }

    pub async fn list_object_versions(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> StoreResult<Vec<(String, Vec<ObjectVersion>)>> {
        self.buckets
            .read()
            .await
            .get(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;

        let objects = self.objects.read().await;
        let mut result: Vec<(String, Vec<ObjectVersion>)> = objects
            .iter()
            .filter(|((b, k), _)| b == bucket && k.starts_with(prefix))
            .map(|((_, k), vs)| (k.clone(), vs.clone()))
            .collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(result)
    }

    pub async fn delete_objects(
        &self,
        bucket: &str,
        objects: Vec<DeleteObjectEntry>,
    ) -> StoreResult<Vec<DeleteObjectEntryResult>> {
        let mut results = Vec::new();
        for entry in objects {
            match self
                .delete_object(bucket, &entry.key, entry.version_id.as_deref())
                .await
            {
                Ok(r) => results.push(DeleteObjectEntryResult {
                    key: entry.key,
                    version_id: r.version_id,
                    delete_marker: r.delete_marker,
                    error: None,
                }),
                Err(e) => results.push(DeleteObjectEntryResult {
                    key: entry.key,
                    version_id: None,
                    delete_marker: false,
                    error: Some((e.s3_code().to_string(), e.to_string())),
                }),
            }
        }
        Ok(results)
    }

    pub async fn put_object_tagging(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
        tags: HashMap<String, String>,
    ) -> StoreResult<()> {
        let mut objects = self.objects.write().await;
        let versions = objects
            .get_mut(&(bucket.to_string(), key.to_string()))
            .ok_or_else(|| StoreError::ObjectNotFound {
                bucket: bucket.to_string(),
                key: key.to_string(),
            })?;
        let version = if let Some(vid) = version_id {
            versions.iter_mut().rev().find(|v| v.version_id.as_deref() == Some(vid))
        } else {
            versions.last_mut()
        }
        .ok_or_else(|| StoreError::ObjectNotFound {
            bucket: bucket.to_string(),
            key: key.to_string(),
        })?;
        version.tags = tags;
        Ok(())
    }

    pub async fn get_object_tagging(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
    ) -> StoreResult<HashMap<String, String>> {
        let objects = self.objects.read().await;
        let versions = objects
            .get(&(bucket.to_string(), key.to_string()))
            .ok_or_else(|| StoreError::ObjectNotFound {
                bucket: bucket.to_string(),
                key: key.to_string(),
            })?;
        let version = if let Some(vid) = version_id {
            versions.iter().rev().find(|v| v.version_id.as_deref() == Some(vid))
        } else {
            versions.last()
        }
        .ok_or_else(|| StoreError::ObjectNotFound {
            bucket: bucket.to_string(),
            key: key.to_string(),
        })?;
        Ok(version.tags.clone())
    }

    // ── Multipart upload ────────────────────────────────────────────────────────

    pub async fn create_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        content_type: &str,
        metadata: HashMap<String, String>,
    ) -> StoreResult<String> {
        self.buckets
            .read()
            .await
            .get(bucket)
            .ok_or_else(|| StoreError::BucketNotFound(bucket.to_string()))?;

        let upload_id = Uuid::new_v4().to_string();
        let meta_json = serde_json::to_string(&metadata)?;
        self.multiparts.write().await.insert(
            upload_id.clone(),
            MultipartUpload {
                upload_id: upload_id.clone(),
                bucket: bucket.to_string(),
                key: key.to_string(),
                initiated: Utc::now(),
                owner: "cave".to_string(),
                content_type: content_type.to_string(),
                metadata,
                parts: HashMap::new(),
            },
        );
        self.wal
            .append(&WalEntry::MultipartInit {
                upload_id: upload_id.clone(),
                bucket: bucket.to_string(),
                key: key.to_string(),
                metadata_json: meta_json,
            })
            .await?;
        Ok(upload_id)
    }

    pub async fn upload_part(
        &self,
        upload_id: &str,
        part_number: u32,
        data: Vec<u8>,
    ) -> StoreResult<String> {
        if part_number < 1 || part_number > 10000 {
            return Err(StoreError::InvalidPart(format!(
                "part number {part_number} out of range [1, 10000]"
            )));
        }
        let (bucket, key) = {
            let multiparts = self.multiparts.read().await;
            let mp = multiparts
                .get(upload_id)
                .ok_or_else(|| StoreError::NoSuchUpload(upload_id.to_string()))?;
            (mp.bucket.clone(), mp.key.clone())
        };

        let etag = compute_etag(&data);
        let size = data.len() as u64;
        let rel_path = format!(
            "{}/.multipart/{}/{}.part",
            bucket, upload_id, part_number
        );
        let abs_path = self.data_dir.join(&rel_path);
        if let Some(p) = abs_path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&abs_path, &data)?;

        self.multiparts
            .write()
            .await
            .get_mut(upload_id)
            .ok_or_else(|| StoreError::NoSuchUpload(upload_id.to_string()))?
            .parts
            .insert(
                part_number,
                UploadedPart {
                    part_number,
                    etag: etag.clone(),
                    size,
                    storage_path: rel_path.clone(),
                    last_modified: Utc::now(),
                },
            );

        self.wal
            .append(&WalEntry::MultipartPart {
                upload_id: upload_id.to_string(),
                part_number,
                etag: etag.clone(),
                size,
                storage_path: rel_path,
            })
            .await?;
        Ok(etag)
    }

    pub async fn complete_multipart_upload(
        &self,
        upload_id: &str,
        parts: Vec<(u32, String)>, // (part_number, etag)
    ) -> StoreResult<CompleteMultipartResult> {
        let mp = self
            .multiparts
            .read()
            .await
            .get(upload_id)
            .cloned()
            .ok_or_else(|| StoreError::NoSuchUpload(upload_id.to_string()))?;

        // Validate parts: must be ordered, all present, etags match
        if parts.is_empty() {
            return Err(StoreError::InvalidPart("no parts provided".into()));
        }
        for (i, (pn, _)) in parts.iter().enumerate() {
            if i > 0 && *pn <= parts[i - 1].0 {
                return Err(StoreError::InvalidPart("parts must be in ascending order".into()));
            }
        }

        // Assemble parts into final object
        let mut assembled = Vec::new();
        for (pn, expected_etag) in &parts {
            let part = mp.parts.get(pn).ok_or_else(|| {
                StoreError::InvalidPart(format!("part {pn} not found"))
            })?;
            if &part.etag != expected_etag {
                return Err(StoreError::InvalidPart(format!(
                    "part {pn} etag mismatch: expected {expected_etag}, got {}",
                    part.etag
                )));
            }
            // Parts < 5 MB are only valid for the last part
            if part.size < 5 * 1024 * 1024 && pn != &parts.last().unwrap().0 {
                return Err(StoreError::EntityTooSmall);
            }
            let data = std::fs::read(self.data_dir.join(&part.storage_path))?;
            assembled.extend_from_slice(&data);
        }

        // Compute final ETag (AWS uses MD5 of part ETags concatenated)
        let part_etags: String = parts.iter().map(|(_, e)| e.as_str()).collect::<Vec<_>>().join("");
        let final_etag = format!("{}-{}", compute_etag(part_etags.as_bytes()), parts.len());

        let versioning = self
            .buckets
            .read()
            .await
            .get(&mp.bucket)
            .map(|b| b.versioning.clone())
            .unwrap_or(VersioningState::Disabled);

        let version_id = match versioning {
            VersioningState::Enabled => Some(Uuid::new_v4().to_string()),
            _ => None,
        };

        let rel_path = object_path(&mp.bucket, &mp.key, version_id.as_deref());
        let abs_path = self.data_dir.join(&rel_path);
        if let Some(p) = abs_path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&abs_path, &assembled)?;

        let size = assembled.len() as u64;
        let version = ObjectVersion {
            version_id: version_id.clone(),
            etag: final_etag.clone(),
            size,
            last_modified: Utc::now(),
            content_type: mp.content_type.clone(),
            metadata: mp.metadata.clone(),
            tags: HashMap::new(),
            storage_class: StorageClass::Standard,
            storage_path: rel_path.to_string_lossy().to_string(),
            encryption: None,
            delete_marker: false,
            restore_status: None,
        };

        self.objects
            .write()
            .await
            .entry((mp.bucket.clone(), mp.key.clone()))
            .or_default()
            .push(version);

        // Clean up part files
        for part in mp.parts.values() {
            let _ = std::fs::remove_file(self.data_dir.join(&part.storage_path));
        }

        self.multiparts.write().await.remove(upload_id);

        self.wal
            .append(&WalEntry::MultipartComplete {
                upload_id: upload_id.to_string(),
                final_etag: final_etag.clone(),
                final_path: rel_path.to_string_lossy().to_string(),
                version_id: version_id.clone(),
            })
            .await?;

        let notification_cfg = self
            .buckets
            .read()
            .await
            .get(&mp.bucket)
            .map(|b| b.notification_config.clone())
            .unwrap_or_default();
        let event = S3Event::object_created_multipart(&mp.bucket, &mp.key, size, &final_etag);
        notification::dispatch(&notification_cfg, &event, &self.event_tx).await;

        Ok(CompleteMultipartResult {
            bucket: mp.bucket,
            key: mp.key,
            etag: final_etag,
            version_id,
        })
    }

    pub async fn abort_multipart_upload(&self, upload_id: &str) -> StoreResult<()> {
        let mp = self
            .multiparts
            .write()
            .await
            .remove(upload_id)
            .ok_or_else(|| StoreError::NoSuchUpload(upload_id.to_string()))?;

        // Clean up part files
        for part in mp.parts.values() {
            let _ = std::fs::remove_file(self.data_dir.join(&part.storage_path));
        }
        self.wal
            .append(&WalEntry::MultipartAbort {
                upload_id: upload_id.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn list_multipart_uploads(&self, bucket: &str) -> Vec<MultipartUpload> {
        self.multiparts
            .read()
            .await
            .values()
            .filter(|m| m.bucket == bucket)
            .cloned()
            .collect()
    }

    pub async fn list_parts(&self, upload_id: &str) -> StoreResult<Vec<UploadedPart>> {
        let multiparts = self.multiparts.read().await;
        let mp = multiparts
            .get(upload_id)
            .ok_or_else(|| StoreError::NoSuchUpload(upload_id.to_string()))?;
        let mut parts: Vec<UploadedPart> = mp.parts.values().cloned().collect();
        parts.sort_by_key(|p| p.part_number);
        Ok(parts)
    }

    /// Background lifecycle enforcer.
    pub async fn run_lifecycle_enforcer(store: Arc<ObjectStore>) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let buckets = store.buckets.read().await;
            let bucket_rules: Vec<(String, Vec<LifecycleRule>)> = buckets
                .values()
                .filter(|b| !b.lifecycle_rules.is_empty())
                .map(|b| (b.name.clone(), b.lifecycle_rules.clone()))
                .collect();
            drop(buckets);

            for (bucket_name, rules) in bucket_rules {
                let objects = store.objects.read().await;
                let mut to_delete: Vec<(String, Option<String>)> = Vec::new();

                for ((b, key), versions) in objects.iter() {
                    if b != &bucket_name {
                        continue;
                    }
                    for rule in &rules {
                        for v in versions {
                            if lifecycle::should_expire(rule, key, v) {
                                to_delete.push((key.clone(), v.version_id.clone()));
                            }
                        }
                    }
                }
                drop(objects);

                for (key, version_id) in to_delete {
                    debug!("Lifecycle expiring {bucket_name}/{key}");
                    let _ = store.delete_object(&bucket_name, &key, version_id.as_deref()).await;
                }

                // Abort incomplete multipart uploads
                let multiparts = store.multiparts.read().await;
                let to_abort: Vec<String> = multiparts
                    .values()
                    .filter(|m| m.bucket == bucket_name)
                    .filter(|m| {
                        let bucket_read = store.buckets.try_read().ok();
                        bucket_read
                            .as_ref()
                            .and_then(|bs| bs.get(&bucket_name))
                            .map(|b| {
                                b.lifecycle_rules.iter().any(|rule| {
                                    lifecycle::should_abort_multipart(rule, &m.key, &m.initiated)
                                })
                            })
                            .unwrap_or(false)
                    })
                    .map(|m| m.upload_id.clone())
                    .collect();
                drop(multiparts);

                for upload_id in to_abort {
                    debug!("Lifecycle aborting multipart {upload_id}");
                    let _ = store.abort_multipart_upload(&upload_id).await;
                }
            }
        }
    }
}

// ── Helper types ───────────────────────────────────────────────────────────────

pub struct PutObjectResult {
    pub etag: String,
    pub version_id: Option<String>,
}

pub struct GetObjectResult {
    pub body: Vec<u8>,
    pub content_type: String,
    pub etag: String,
    pub last_modified: chrono::DateTime<Utc>,
    pub size: u64,
    pub version_id: Option<String>,
    pub metadata: HashMap<String, String>,
    pub storage_class: String,
    pub content_range: Option<String>,
    pub encryption: Option<ObjectEncryption>,
    pub delete_marker: bool,
}

pub struct HeadObjectResult {
    pub content_type: String,
    pub etag: String,
    pub last_modified: chrono::DateTime<Utc>,
    pub size: u64,
    pub version_id: Option<String>,
    pub metadata: HashMap<String, String>,
    pub storage_class: String,
    pub delete_marker: bool,
    pub encryption: Option<ObjectEncryption>,
}

pub struct DeleteObjectResult {
    pub version_id: Option<String>,
    pub delete_marker: bool,
}

pub struct CopyObjectResult {
    pub etag: String,
    pub last_modified: chrono::DateTime<Utc>,
    pub version_id: Option<String>,
}

pub struct ObjectListEntry {
    pub key: String,
    pub last_modified: chrono::DateTime<Utc>,
    pub etag: String,
    pub size: u64,
    pub storage_class: String,
}

pub struct ListResult {
    pub contents: Vec<ObjectListEntry>,
    pub common_prefixes: Vec<String>,
    pub is_truncated: bool,
    pub key_count: u32,
    pub next_continuation_token: Option<String>,
}

pub struct DeleteObjectEntry {
    pub key: String,
    pub version_id: Option<String>,
}

pub struct DeleteObjectEntryResult {
    pub key: String,
    pub version_id: Option<String>,
    pub delete_marker: bool,
    pub error: Option<(String, String)>, // (code, message)
}

pub struct CompleteMultipartResult {
    pub bucket: String,
    pub key: String,
    pub etag: String,
    pub version_id: Option<String>,
}

// ── Utilities ──────────────────────────────────────────────────────────────────

fn validate_bucket_name(name: &str) -> StoreResult<()> {
    if name.len() < 3 || name.len() > 63 {
        return Err(StoreError::InvalidBucketName(
            "bucket name must be 3-63 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
    {
        return Err(StoreError::InvalidBucketName(
            "bucket name may only contain lowercase letters, numbers, hyphens, and dots".into(),
        ));
    }
    if name.starts_with('-') || name.ends_with('-') || name.starts_with('.') || name.ends_with('.') {
        return Err(StoreError::InvalidBucketName(
            "bucket name cannot start or end with hyphen or dot".into(),
        ));
    }
    Ok(())
}

fn compute_etag(data: &[u8]) -> String {
    let d = digest::digest(&digest::SHA256, data);
    hex::encode(&d.as_ref()[..16])
}

fn object_path(bucket: &str, key: &str, version_id: Option<&str>) -> PathBuf {
    if let Some(vid) = version_id {
        PathBuf::from(format!("{bucket}/{key}.{vid}"))
    } else {
        PathBuf::from(format!("{bucket}/{key}"))
    }
}

fn contents_last_key(
    objects: &HashMap<(String, String), Vec<ObjectVersion>>,
    bucket: &str,
    prefix: &str,
    max_keys: u32,
    _continuation_token: Option<&str>,
) -> Option<String> {
    let mut keys: Vec<&String> = objects
        .keys()
        .filter(|(b, k)| b == bucket && k.starts_with(prefix))
        .map(|(_, k)| k)
        .collect();
    keys.sort();
    keys.get(max_keys as usize).map(|k| k.to_string())
}
