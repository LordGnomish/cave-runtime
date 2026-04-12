//! In-memory object store: buckets, objects, versioning, multipart uploads.

use crate::models::{
    AccessPolicy, Bucket, LifecycleRule, MultipartUpload, ReplicationRule, StorageObject,
    UploadPart,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

/// The in-memory object store, guarded externally by a Mutex.
pub struct ObjectStore {
    pub buckets: HashMap<String, Bucket>,
    /// Key: (bucket_name, object_key) → list of versions (oldest first).
    pub objects: HashMap<(String, String), Vec<StorageObject>>,
    pub multipart_uploads: HashMap<Uuid, MultipartUpload>,
}

impl ObjectStore {
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
            objects: HashMap::new(),
            multipart_uploads: HashMap::new(),
        }
    }

    // ── Bucket operations ─────────────────────────────────────────────────────

    pub fn create_bucket(
        &mut self,
        name: String,
        region: Option<String>,
        tags: Option<HashMap<String, String>>,
    ) -> Result<Bucket, String> {
        if self.buckets.contains_key(&name) {
            return Err(format!("bucket '{name}' already exists"));
        }
        if !is_valid_bucket_name(&name) {
            return Err(format!("invalid bucket name '{name}'"));
        }
        let bucket = Bucket {
            id: Uuid::new_v4(),
            name: name.clone(),
            region: region.unwrap_or_else(|| "us-east-1".to_string()),
            access_policy: AccessPolicy::default(),
            versioning_enabled: false,
            lifecycle_rules: vec![],
            replication_rules: vec![],
            tags: tags.unwrap_or_default(),
            created_at: Utc::now(),
        };
        self.buckets.insert(name, bucket.clone());
        Ok(bucket)
    }

    pub fn get_bucket(&self, name: &str) -> Option<&Bucket> {
        self.buckets.get(name)
    }

    pub fn list_buckets(&self) -> Vec<Bucket> {
        let mut v: Vec<Bucket> = self.buckets.values().cloned().collect();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        v
    }

    pub fn delete_bucket(&mut self, name: &str) -> Result<(), String> {
        if !self.buckets.contains_key(name) {
            return Err(format!("bucket '{name}' not found"));
        }
        let has_objects = self.objects.keys().any(|(b, _)| b.as_str() == name);
        if has_objects {
            return Err(format!("bucket '{name}' is not empty"));
        }
        self.buckets.remove(name);
        Ok(())
    }

    pub fn set_versioning(&mut self, bucket: &str, enabled: bool) -> Result<(), String> {
        let b = self
            .buckets
            .get_mut(bucket)
            .ok_or_else(|| format!("bucket '{bucket}' not found"))?;
        b.versioning_enabled = enabled;
        Ok(())
    }

    pub fn set_lifecycle_rules(
        &mut self,
        bucket: &str,
        rules: Vec<LifecycleRule>,
    ) -> Result<(), String> {
        let b = self
            .buckets
            .get_mut(bucket)
            .ok_or_else(|| format!("bucket '{bucket}' not found"))?;
        b.lifecycle_rules = rules;
        Ok(())
    }

    pub fn set_access_policy(
        &mut self,
        bucket: &str,
        policy: AccessPolicy,
    ) -> Result<(), String> {
        let b = self
            .buckets
            .get_mut(bucket)
            .ok_or_else(|| format!("bucket '{bucket}' not found"))?;
        b.access_policy = policy;
        Ok(())
    }

    pub fn set_replication_rules(
        &mut self,
        bucket: &str,
        rules: Vec<ReplicationRule>,
    ) -> Result<(), String> {
        let b = self
            .buckets
            .get_mut(bucket)
            .ok_or_else(|| format!("bucket '{bucket}' not found"))?;
        b.replication_rules = rules;
        Ok(())
    }

    // ── Object operations ─────────────────────────────────────────────────────

    pub fn put_object(
        &mut self,
        bucket: &str,
        key: String,
        content: serde_json::Value,
        content_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<StorageObject, String> {
        if !self.buckets.contains_key(bucket) {
            return Err(format!("bucket '{bucket}' not found"));
        }
        // Read versioning flag without holding a borrow into self.buckets.
        let versioning = self.buckets[bucket].versioning_enabled;

        let content_str = serde_json::to_string(&content).unwrap_or_default();
        let size = content_str.len() as u64;
        let etag = Uuid::new_v4().to_string();
        let version_id = if versioning { Some(Uuid::new_v4()) } else { None };

        let obj = StorageObject {
            key: key.clone(),
            bucket: bucket.to_string(),
            size,
            content_type: content_type
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            metadata: metadata.unwrap_or_default(),
            etag,
            version_id,
            is_delete_marker: false,
            last_modified: Utc::now(),
            content,
        };

        let versions = self
            .objects
            .entry((bucket.to_string(), key))
            .or_default();
        if !versioning {
            versions.clear();
        }
        versions.push(obj.clone());
        Ok(obj)
    }

    /// Get the current (latest non-delete-marker) version of an object.
    pub fn get_object(
        &self,
        bucket: &str,
        key: &str,
        version_id: Option<Uuid>,
    ) -> Option<StorageObject> {
        let versions = self.objects.get(&(bucket.to_string(), key.to_string()))?;
        let obj = if let Some(vid) = version_id {
            versions.iter().find(|o| o.version_id == Some(vid))
        } else {
            versions.iter().rev().find(|o| !o.is_delete_marker)
        };
        obj.cloned()
    }

    pub fn delete_object(
        &mut self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<Uuid>, String> {
        if !self.buckets.contains_key(bucket) {
            return Err(format!("bucket '{bucket}' not found"));
        }
        let versioning = self.buckets[bucket].versioning_enabled;

        if versioning {
            let marker_id = Uuid::new_v4();
            let marker = StorageObject {
                key: key.to_string(),
                bucket: bucket.to_string(),
                size: 0,
                content_type: String::new(),
                metadata: HashMap::new(),
                etag: Uuid::new_v4().to_string(),
                version_id: Some(marker_id),
                is_delete_marker: true,
                last_modified: Utc::now(),
                content: serde_json::Value::Null,
            };
            self.objects
                .entry((bucket.to_string(), key.to_string()))
                .or_default()
                .push(marker);
            Ok(Some(marker_id))
        } else {
            self.objects.remove(&(bucket.to_string(), key.to_string()));
            Ok(None)
        }
    }

    pub fn list_objects(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        max_keys: Option<usize>,
    ) -> Result<Vec<StorageObject>, String> {
        if !self.buckets.contains_key(bucket) {
            return Err(format!("bucket '{bucket}' not found"));
        }
        let mut objects: Vec<StorageObject> = self
            .objects
            .iter()
            .filter(|((b, k), _)| {
                b.as_str() == bucket
                    && prefix.map_or(true, |p| k.starts_with(p))
            })
            .filter_map(|(_, versions)| {
                versions.iter().rev().find(|o| !o.is_delete_marker).cloned()
            })
            .collect();
        objects.sort_by(|a, b| a.key.cmp(&b.key));
        if let Some(max) = max_keys {
            objects.truncate(max);
        }
        Ok(objects)
    }

    pub fn list_object_versions(&self, bucket: &str, key: &str) -> Vec<StorageObject> {
        self.objects
            .get(&(bucket.to_string(), key.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    // ── Multipart upload ──────────────────────────────────────────────────────

    pub fn initiate_multipart(
        &mut self,
        bucket: &str,
        key: String,
        content_type: Option<String>,
        metadata: Option<HashMap<String, String>>,
    ) -> Result<Uuid, String> {
        if !self.buckets.contains_key(bucket) {
            return Err(format!("bucket '{bucket}' not found"));
        }
        let upload_id = Uuid::new_v4();
        self.multipart_uploads.insert(
            upload_id,
            MultipartUpload {
                upload_id,
                bucket: bucket.to_string(),
                key,
                parts: vec![],
                initiated_at: Utc::now(),
                content_type: content_type
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                metadata: metadata.unwrap_or_default(),
            },
        );
        Ok(upload_id)
    }

    pub fn upload_part(
        &mut self,
        upload_id: Uuid,
        part_number: u32,
        content: serde_json::Value,
    ) -> Result<String, String> {
        let upload = self
            .multipart_uploads
            .get_mut(&upload_id)
            .ok_or_else(|| format!("upload '{upload_id}' not found"))?;
        let content_str = serde_json::to_string(&content).unwrap_or_default();
        let size = content_str.len() as u64;
        let etag = Uuid::new_v4().to_string();
        upload.parts.retain(|p| p.part_number != part_number);
        upload.parts.push(UploadPart {
            part_number,
            etag: etag.clone(),
            size,
            content,
        });
        upload.parts.sort_by_key(|p| p.part_number);
        Ok(etag)
    }

    pub fn complete_multipart(
        &mut self,
        upload_id: Uuid,
    ) -> Result<StorageObject, String> {
        let upload = self
            .multipart_uploads
            .remove(&upload_id)
            .ok_or_else(|| format!("upload '{upload_id}' not found"))?;

        let versioning = self
            .buckets
            .get(&upload.bucket)
            .map(|b| b.versioning_enabled)
            .unwrap_or(false);

        let total_size: u64 = upload.parts.iter().map(|p| p.size).sum();
        let etag = format!("{}-{}", Uuid::new_v4(), upload.parts.len());
        let merged = serde_json::Value::Array(
            upload.parts.iter().map(|p| p.content.clone()).collect(),
        );

        let obj = StorageObject {
            key: upload.key.clone(),
            bucket: upload.bucket.clone(),
            size: total_size,
            content_type: upload.content_type,
            metadata: upload.metadata,
            etag,
            version_id: if versioning { Some(Uuid::new_v4()) } else { None },
            is_delete_marker: false,
            last_modified: Utc::now(),
            content: merged,
        };

        let versions = self
            .objects
            .entry((obj.bucket.clone(), obj.key.clone()))
            .or_default();
        if !versioning {
            versions.clear();
        }
        versions.push(obj.clone());
        Ok(obj)
    }

    pub fn abort_multipart(&mut self, upload_id: Uuid) -> Result<(), String> {
        self.multipart_uploads
            .remove(&upload_id)
            .ok_or_else(|| format!("upload '{upload_id}' not found"))?;
        Ok(())
    }

    pub fn list_multipart_uploads(&self, bucket: &str) -> Vec<&MultipartUpload> {
        self.multipart_uploads
            .values()
            .filter(|u| u.bucket.as_str() == bucket)
            .collect()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_valid_bucket_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}
