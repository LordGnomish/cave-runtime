//! S3-compatible object store — file-backed with in-memory index.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use aes_gcm::aead::rand_core::RngCore;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use bytes::Bytes;
use chrono::Utc;
use dashmap::DashMap;
use hmac::{Hmac, Mac};
use md5::{Digest as Md5Digest, Md5};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{Result, StoreError};
use super::types::*;

// ─── Internal bucket state ───────────────────────────────────────────────────

#[derive(Debug)]
pub struct BucketState {
    pub info: BucketInfo,
    /// key → vec of ObjectVersion (newest first)
    pub objects: DashMap<String, Vec<ObjectVersion>>,
    pub multipart: DashMap<String, MultipartUpload>,
    pub versioning: parking_lot::RwLock<BucketVersioning>,
    pub lifecycle: parking_lot::RwLock<Vec<LifecycleRule>>,
    pub policy: parking_lot::RwLock<Option<BucketPolicy>>,
    pub notification: parking_lot::RwLock<NotificationConfig>,
    pub acl: parking_lot::RwLock<BucketAcl>,
}

impl BucketState {
    fn new(info: BucketInfo) -> Self {
        Self {
            info,
            objects: DashMap::new(),
            multipart: DashMap::new(),
            versioning: parking_lot::RwLock::new(BucketVersioning::default()),
            lifecycle: parking_lot::RwLock::new(vec![]),
            policy: parking_lot::RwLock::new(None),
            notification: parking_lot::RwLock::new(NotificationConfig::default()),
            acl: parking_lot::RwLock::new(BucketAcl::default()),
        }
    }
}

// ─── S3Store ──────────────────────────────────────────────────────────────────

pub struct S3Store {
    data_dir: PathBuf,
    buckets: Arc<DashMap<String, Arc<BucketState>>>,
    /// 32-byte AES-256 master key for SSE-S3
    sse_master_key: Vec<u8>,
}

impl S3Store {
    pub fn new(data_dir: impl Into<PathBuf>, sse_master_key: Vec<u8>) -> std::io::Result<Self> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir)?;
        let store = Self {
            data_dir,
            buckets: Arc::new(DashMap::new()),
            sse_master_key,
        };
        store.load_from_disk()?;
        Ok(store)
    }

    /// Restore bucket metadata from disk (sidecar .meta.json files).
    fn load_from_disk(&self) -> std::io::Result<()> {
        for entry in std::fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let bucket_name = entry.file_name().to_string_lossy().to_string();
            let meta_path = entry.path().join(".bucket_meta.json");
            if let Ok(data) = std::fs::read(&meta_path) {
                if let Ok(info) = serde_json::from_slice::<BucketInfo>(&data) {
                    let state = Arc::new(BucketState::new(info));
                    // Load object metadata
                    self.load_objects(&entry.path(), &state)?;
                    self.buckets.insert(bucket_name, state);
                }
            }
        }
        Ok(())
    }

    fn load_objects(&self, bucket_dir: &Path, state: &BucketState) -> std::io::Result<()> {
        let meta_dir = bucket_dir.join(".meta");
        if !meta_dir.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(&meta_dir)? {
            let entry = entry?;
            if let Ok(data) = std::fs::read(entry.path()) {
                if let Ok(versions) = serde_json::from_slice::<Vec<ObjectVersion>>(&data) {
                    if let Some(first) = versions.first() {
                        state.objects.insert(first.metadata.key.clone(), versions);
                    }
                }
            }
        }
        Ok(())
    }

    fn bucket(&self, name: &str) -> Result<Arc<BucketState>> {
        self.buckets.get(name)
            .map(|b| Arc::clone(&*b))
            .ok_or_else(|| StoreError::BucketNotFound(name.to_string()))
    }

    fn object_data_path(&self, bucket: &str, key: &str, version_id: Option<&str>) -> PathBuf {
        let safe_key = key.replace('/', "_SLASH_");
        let version = version_id.unwrap_or("latest");
        self.data_dir
            .join(bucket)
            .join(format!("{safe_key}.{version}.dat"))
    }

    fn object_meta_path(&self, bucket: &str, key: &str) -> PathBuf {
        let safe_key = key.replace('/', "_SLASH_");
        self.data_dir
            .join(bucket)
            .join(".meta")
            .join(format!("{safe_key}.json"))
    }

    fn save_object_meta(&self, bucket: &str, key: &str, versions: &[ObjectVersion]) -> Result<()> {
        let meta_path = self.object_meta_path(bucket, key);
        std::fs::create_dir_all(meta_path.parent().unwrap())?;
        let data = serde_json::to_vec(versions)?;
        std::fs::write(&meta_path, data)?;
        Ok(())
    }

    fn etag_of(data: &[u8]) -> String {
        let mut hasher = Md5::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    fn encrypt_sse_s3(&self, data: &[u8]) -> Result<Vec<u8>> {
        self.aes_gcm_encrypt(data, &self.sse_master_key)
    }

    fn decrypt_sse_s3(&self, data: &[u8]) -> Result<Vec<u8>> {
        self.aes_gcm_decrypt(data, &self.sse_master_key)
    }

    fn encrypt_sse_c(&self, data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
        self.aes_gcm_encrypt(data, key)
    }

    fn decrypt_sse_c(&self, data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
        self.aes_gcm_decrypt(data, key)
    }

    /// AES-256-GCM encrypt: returns [12-byte nonce || ciphertext+tag]
    fn aes_gcm_encrypt(&self, data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
        if key.len() != 32 {
            return Err(StoreError::InvalidRequest("SSE key must be 32 bytes".into()));
        }
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, data)
            .map_err(|e| StoreError::InvalidRequest(format!("AES-GCM encrypt: {e}")))?;
        let mut out = nonce_bytes.to_vec();
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// AES-256-GCM decrypt: expects [12-byte nonce || ciphertext+tag]
    fn aes_gcm_decrypt(&self, data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 12 {
            return Err(StoreError::WalCorrupted("encrypted data too short".into()));
        }
        if key.len() != 32 {
            return Err(StoreError::InvalidRequest("SSE key must be 32 bytes".into()));
        }
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
        let nonce = Nonce::from_slice(&data[..12]);
        let plaintext = cipher
            .decrypt(nonce, &data[12..])
            .map_err(|e| StoreError::InvalidRequest(format!("AES-GCM decrypt: {e}")))?;
        Ok(plaintext)
    }

    // ── Bucket operations ─────────────────────────────────────────────────────

    pub fn create_bucket(&self, name: String, region: String) -> Result<()> {
        if self.buckets.contains_key(&name) {
            return Err(StoreError::BucketExists(name));
        }
        validate_bucket_name(&name)?;
        let dir = self.data_dir.join(&name);
        std::fs::create_dir_all(&dir)?;
        let info = BucketInfo { name: name.clone(), creation_date: Utc::now(), region };
        let data = serde_json::to_vec(&info)?;
        std::fs::write(dir.join(".bucket_meta.json"), data)?;
        self.buckets.insert(name, Arc::new(BucketState::new(info)));
        Ok(())
    }

    pub fn delete_bucket(&self, name: &str) -> Result<()> {
        let state = self.bucket(name)?;
        if !state.objects.is_empty() {
            return Err(StoreError::InvalidRequest("bucket is not empty".into()));
        }
        self.buckets.remove(name);
        let dir = self.data_dir.join(name);
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    pub fn list_buckets(&self) -> Vec<BucketInfo> {
        let mut buckets: Vec<BucketInfo> = self.buckets
            .iter()
            .map(|e| e.value().info.clone())
            .collect();
        buckets.sort_by(|a, b| a.name.cmp(&b.name));
        buckets
    }

    pub fn head_bucket(&self, name: &str) -> Result<BucketInfo> {
        self.bucket(name).map(|b| b.info.clone())
    }

    // ── Object operations ─────────────────────────────────────────────────────

    pub fn put_object(
        &self,
        bucket: &str,
        key: &str,
        data: Bytes,
        metadata: HashMap<String, String>,
        content_type: Option<String>,
        sse: Option<SseConfig>,
    ) -> Result<ObjectMetadata> {
        let state = self.bucket(bucket)?;
        let version_id = {
            let v = state.versioning.read();
            if v.status == VersioningStatus::Enabled {
                Some(Uuid::new_v4().to_string())
            } else {
                None
            }
        };

        let etag = Self::etag_of(&data);
        let sse_algorithm = sse.as_ref().map(|s| s.algorithm.clone());

        // Normalise version_id: use UUID if versioning enabled, else "latest".
        let version_string = version_id.clone().unwrap_or_else(|| "latest".to_string());

        // Encrypt if needed
        let stored_data = match &sse {
            Some(s) if s.algorithm == SseAlgorithm::AwsS3 => self.encrypt_sse_s3(&data)?,
            Some(s) if s.algorithm == SseAlgorithm::Customer => {
                let key = s.customer_key.as_ref()
                    .ok_or_else(|| StoreError::InvalidRequest("SSE-C requires customer key".into()))?;
                self.encrypt_sse_c(&data, key)?
            }
            _ => data.to_vec(),
        };

        let data_path = self.object_data_path(bucket, key, Some(&version_string));
        std::fs::create_dir_all(data_path.parent().unwrap())?;
        std::fs::write(&data_path, &stored_data)?;

        let obj_meta = ObjectMetadata {
            key: key.to_string(),
            size: data.len() as u64,
            etag,
            last_modified: Utc::now(),
            content_type: content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
            storage_class: "STANDARD".to_string(),
            version_id: version_id.clone(),
            user_metadata: metadata,
            sse_algorithm,
        };

        let version = ObjectVersion {
            version_id: version_string.clone(),
            is_delete_marker: false,
            metadata: obj_meta.clone(),
        };

        let mut entry = state.objects.entry(key.to_string()).or_default();
        entry.insert(0, version);

        self.save_object_meta(bucket, key, &entry)?;
        Ok(obj_meta)
    }

    pub fn get_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<&str>,
        range: Option<(u64, u64)>,
        customer_key: Option<&[u8]>,
    ) -> Result<(ObjectMetadata, Bytes)> {
        let state = self.bucket(bucket)?;
        let versions = state.objects.get(key)
            .ok_or_else(|| StoreError::ObjectNotFound(bucket.to_string(), key.to_string()))?;

        let version = find_version(&versions, version_id)?;
        let meta = version.metadata.clone();

        let data_path = self.object_data_path(bucket, key, Some(&version.version_id));
        let raw = std::fs::read(&data_path)?;

        let plaintext = match &meta.sse_algorithm {
            Some(SseAlgorithm::AwsS3) => self.decrypt_sse_s3(&raw)?,
            Some(SseAlgorithm::Customer) => {
                let key = customer_key
                    .ok_or_else(|| StoreError::InvalidRequest("SSE-C: customer key required for get".into()))?;
                self.decrypt_sse_c(&raw, key)?
            }
            _ => raw,
        };

        let data = if let Some((start, end)) = range {
            let end = end.min(plaintext.len() as u64 - 1);
            Bytes::copy_from_slice(&plaintext[start as usize..=end as usize])
        } else {
            Bytes::from(plaintext)
        };

        Ok((meta, data))
    }

    pub fn head_object(&self, bucket: &str, key: &str, version_id: Option<&str>) -> Result<ObjectMetadata> {
        let state = self.bucket(bucket)?;
        let versions = state.objects.get(key)
            .ok_or_else(|| StoreError::ObjectNotFound(bucket.to_string(), key.to_string()))?;
        let version = find_version(&versions, version_id)?;
        Ok(version.metadata.clone())
    }

    pub fn delete_object(&self, bucket: &str, key: &str, version_id: Option<&str>) -> Result<()> {
        let state = self.bucket(bucket)?;
        {
            let v = state.versioning.read();
            if v.status == VersioningStatus::Enabled && version_id.is_none() {
                // Add a delete marker
                let mut entry = state.objects.entry(key.to_string()).or_default();
                let marker = ObjectVersion {
                    version_id: Uuid::new_v4().to_string(),
                    is_delete_marker: true,
                    metadata: ObjectMetadata {
                        key: key.to_string(),
                        size: 0,
                        etag: String::new(),
                        last_modified: Utc::now(),
                        content_type: String::new(),
                        storage_class: "STANDARD".to_string(),
                        version_id: None,
                        user_metadata: HashMap::new(),
                        sse_algorithm: None,
                    },
                };
                entry.insert(0, marker);
                return Ok(());
            }
        }

        let mut versions = state.objects.entry(key.to_string()).or_default();
        if let Some(vid) = version_id {
            let pos = versions.iter().position(|v| v.version_id == vid)
                .ok_or_else(|| StoreError::ObjectNotFound(bucket.to_string(), key.to_string()))?;
            let v = versions.remove(pos);
            let data_path = self.object_data_path(bucket, key, Some(&v.version_id));
            let _ = std::fs::remove_file(data_path);
        } else {
            // Non-versioned: remove the most recent live version.
            if let Some(pos) = versions.iter().position(|v| !v.is_delete_marker) {
                let v = versions.remove(pos);
                let data_path = self.object_data_path(bucket, key, Some(&v.version_id));
                let _ = std::fs::remove_file(data_path);
            }
        }
        if versions.is_empty() {
            drop(versions);
            state.objects.remove(key);
            let meta_path = self.object_meta_path(bucket, key);
            let _ = std::fs::remove_file(meta_path);
        }
        Ok(())
    }

    pub fn copy_object(
        &self,
        src_bucket: &str,
        src_key: &str,
        dst_bucket: &str,
        dst_key: &str,
    ) -> Result<ObjectMetadata> {
        let (src_meta, data) = self.get_object(src_bucket, src_key, None, None, None)?;
        let meta = src_meta.user_metadata.clone();
        let ct = Some(src_meta.content_type.clone());
        self.put_object(dst_bucket, dst_key, data, meta, ct, None)
    }

    pub fn list_objects_v2(
        &self,
        bucket: &str,
        prefix: &str,
        delimiter: &str,
        continuation_token: Option<&str>,
        max_keys: u32,
    ) -> Result<ListObjectsV2Result> {
        let state = self.bucket(bucket)?;
        let max = if max_keys == 0 { 1000 } else { max_keys } as usize;

        // Collect all live objects
        let mut all_keys: Vec<String> = state.objects
            .iter()
            .filter(|e| {
                let v = e.value();
                !v.is_empty() && !v[0].is_delete_marker
            })
            .map(|e| e.key().clone())
            .filter(|k| k.starts_with(prefix))
            .collect();
        all_keys.sort();

        // Apply continuation token (key-based pagination)
        let start = if let Some(token) = continuation_token {
            let decoded = String::from_utf8(
                B64.decode(token).unwrap_or_default()
            ).unwrap_or_default();
            all_keys.iter().position(|k| k.as_str() > decoded.as_str()).unwrap_or(all_keys.len())
        } else {
            0
        };

        let mut contents = Vec::new();
        let mut common_prefixes: Vec<String> = Vec::new();
        let mut count = 0usize;
        let mut last_key = String::new();

        for key in &all_keys[start..] {
            if count >= max {
                break;
            }
            if !delimiter.is_empty() {
                let suffix = &key[prefix.len()..];
                if let Some(pos) = suffix.find(delimiter) {
                    let cp = format!("{}{}{}", prefix, &suffix[..=pos], "");
                    let cp = format!("{}{}", prefix, &suffix[..pos + delimiter.len()]);
                    if !common_prefixes.contains(&cp) {
                        common_prefixes.push(cp);
                        count += 1;
                    }
                    continue;
                }
            }
            let versions = state.objects.get(key).unwrap();
            contents.push(versions[0].metadata.clone());
            last_key = key.clone();
            count += 1;
        }

        let truncated = start + count < all_keys.len();
        let next_token = if truncated {
            Some(B64.encode(last_key.as_bytes()))
        } else {
            None
        };

        Ok(ListObjectsV2Result {
            bucket: bucket.to_string(),
            prefix: prefix.to_string(),
            delimiter: delimiter.to_string(),
            max_keys: max_keys,
            key_count: count as u32,
            truncated,
            next_continuation_token: next_token,
            contents,
            common_prefixes,
        })
    }

    // ── Multipart upload ──────────────────────────────────────────────────────

    pub fn create_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        metadata: HashMap<String, String>,
        content_type: Option<String>,
    ) -> Result<String> {
        let state = self.bucket(bucket)?;
        let upload_id = Uuid::new_v4().to_string();
        let upload = MultipartUpload {
            upload_id: upload_id.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            initiated: Utc::now(),
            parts: BTreeMap::new(),
            metadata,
            content_type: content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
        };
        state.multipart.insert(upload_id.clone(), upload);
        Ok(upload_id)
    }

    pub fn upload_part(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        part_number: u32,
        data: Bytes,
    ) -> Result<String> {
        let state = self.bucket(bucket)?;
        let mut upload = state.multipart.get_mut(upload_id)
            .ok_or_else(|| StoreError::UploadNotFound(upload_id.to_string()))?;

        if upload.key != key {
            return Err(StoreError::InvalidRequest("key mismatch".into()));
        }

        let etag = Self::etag_of(&data);
        let size = data.len() as u64;

        // Store part data
        let part_path = self.data_dir
            .join(bucket)
            .join(format!(".mp_{upload_id}_{part_number}.dat"));
        std::fs::create_dir_all(part_path.parent().unwrap())?;
        std::fs::write(&part_path, &data)?;

        upload.parts.insert(part_number, UploadedPart { part_number, etag: etag.clone(), size });
        Ok(etag)
    }

    pub fn complete_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        parts: Vec<(u32, String)>, // (part_number, etag)
    ) -> Result<ObjectMetadata> {
        let state = self.bucket(bucket)?;
        let upload = state.multipart.get(upload_id)
            .ok_or_else(|| StoreError::UploadNotFound(upload_id.to_string()))?;

        // Assemble parts in order
        let mut assembled = Vec::new();
        for (part_num, _etag) in &parts {
            let part_path = self.data_dir
                .join(bucket)
                .join(format!(".mp_{upload_id}_{part_num}.dat"));
            let data = std::fs::read(&part_path)?;
            assembled.extend_from_slice(&data);
            let _ = std::fs::remove_file(part_path);
        }

        let meta_clone = HashMap::new();
        let ct = Some(upload.content_type.clone());
        drop(upload);
        state.multipart.remove(upload_id);

        self.put_object(bucket, key, Bytes::from(assembled), meta_clone, ct, None)
    }

    pub fn abort_multipart_upload(&self, bucket: &str, key: &str, upload_id: &str) -> Result<()> {
        let state = self.bucket(bucket)?;
        let upload = state.multipart.remove(upload_id)
            .ok_or_else(|| StoreError::UploadNotFound(upload_id.to_string()))?;

        // Clean up part files
        for part_num in upload.1.parts.keys() {
            let part_path = self.data_dir
                .join(bucket)
                .join(format!(".mp_{upload_id}_{part_num}.dat"));
            let _ = std::fs::remove_file(part_path);
        }
        Ok(())
    }

    pub fn list_multipart_uploads(&self, bucket: &str) -> Result<Vec<MultipartUpload>> {
        let state = self.bucket(bucket)?;
        let mut uploads: Vec<MultipartUpload> = state.multipart
            .iter()
            .map(|e| e.value().clone())
            .collect();
        uploads.sort_by(|a, b| a.initiated.cmp(&b.initiated));
        Ok(uploads)
    }

    // ── Presigned URLs ────────────────────────────────────────────────────────

    pub fn presign_url(
        &self,
        bucket: &str,
        key: &str,
        method: &str,
        expires_in_secs: u64,
        base_url: &str,
    ) -> Result<String> {
        self.bucket(bucket)?;
        let expiry = chrono::Utc::now().timestamp() as u64 + expires_in_secs;
        let string_to_sign = format!("{method}\n{bucket}\n{key}\n{expiry}");
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(&self.sse_master_key)
            .map_err(|e| StoreError::InvalidRequest(format!("HMAC error: {e}")))?;
        hmac::Mac::update(&mut mac, string_to_sign.as_bytes());
        let sig = hex::encode(hmac::Mac::finalize(mac).into_bytes());
        Ok(format!(
            "{base_url}/{bucket}/{key}?X-Amz-Expires={expires_in_secs}&X-Amz-Expires-At={expiry}&X-Amz-Signature={sig}"
        ))
    }

    // ── Versioning ────────────────────────────────────────────────────────────

    pub fn get_bucket_versioning(&self, bucket: &str) -> Result<BucketVersioning> {
        Ok(self.bucket(bucket)?.versioning.read().clone())
    }

    pub fn put_bucket_versioning(&self, bucket: &str, config: BucketVersioning) -> Result<()> {
        *self.bucket(bucket)?.versioning.write() = config;
        Ok(())
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    pub fn get_bucket_lifecycle(&self, bucket: &str) -> Result<Vec<LifecycleRule>> {
        Ok(self.bucket(bucket)?.lifecycle.read().clone())
    }

    pub fn put_bucket_lifecycle(&self, bucket: &str, rules: Vec<LifecycleRule>) -> Result<()> {
        *self.bucket(bucket)?.lifecycle.write() = rules;
        Ok(())
    }

    // ── Policy ────────────────────────────────────────────────────────────────

    pub fn get_bucket_policy(&self, bucket: &str) -> Result<BucketPolicy> {
        self.bucket(bucket)?
            .policy.read()
            .clone()
            .ok_or_else(|| StoreError::InvalidRequest("no bucket policy".into()))
    }

    pub fn put_bucket_policy(&self, bucket: &str, policy: BucketPolicy) -> Result<()> {
        *self.bucket(bucket)?.policy.write() = Some(policy);
        Ok(())
    }

    pub fn delete_bucket_policy(&self, bucket: &str) -> Result<()> {
        *self.bucket(bucket)?.policy.write() = None;
        Ok(())
    }

    // ── Notification ──────────────────────────────────────────────────────────

    pub fn get_bucket_notification(&self, bucket: &str) -> Result<NotificationConfig> {
        Ok(self.bucket(bucket)?.notification.read().clone())
    }

    pub fn put_bucket_notification(&self, bucket: &str, config: NotificationConfig) -> Result<()> {
        *self.bucket(bucket)?.notification.write() = config;
        Ok(())
    }

    // ── ACL ───────────────────────────────────────────────────────────────────

    pub fn get_bucket_acl(&self, bucket: &str) -> Result<BucketAcl> {
        Ok(self.bucket(bucket)?.acl.read().clone())
    }

    pub fn put_bucket_acl(&self, bucket: &str, acl: BucketAcl) -> Result<()> {
        *self.bucket(bucket)?.acl.write() = acl;
        Ok(())
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn find_version<'a>(
    versions: &'a dashmap::mapref::one::Ref<'_, String, Vec<ObjectVersion>>,
    version_id: Option<&str>,
) -> Result<&'a ObjectVersion> {
    match version_id {
        None => versions.iter()
            .find(|v| !v.is_delete_marker)
            .ok_or(StoreError::KeyNotFound),
        Some(vid) => versions.iter()
            .find(|v| v.version_id == vid)
            .ok_or(StoreError::KeyNotFound),
    }
}

fn validate_bucket_name(name: &str) -> Result<()> {
    if name.len() < 3 || name.len() > 63 {
        return Err(StoreError::InvalidRequest(
            "bucket name must be 3-63 characters".into(),
        ));
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.') {
        return Err(StoreError::InvalidRequest(
            "bucket name must contain only lowercase letters, digits, hyphens, and dots".into(),
        ));
    }
    Ok(())
}

use std::collections::BTreeMap;
