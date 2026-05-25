// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use std::collections::HashMap;
use chrono::Utc;
use uuid::Uuid;
use crate::error::{StoreError, StoreResult};
use crate::store::ObjectStore;
use crate::types::{MultipartUpload, ObjectInfo, UploadPart};
use crate::versioning::compute_etag;

pub struct PartInfo {
    pub part_number: u32,
    pub etag: String,
    pub size: u64,
    pub last_modified: chrono::DateTime<chrono::Utc>,
}

impl ObjectStore {
    pub async fn create_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        metadata: HashMap<String, String>,
    ) -> StoreResult<String> {
        // Ensure bucket exists
        {
            let buckets = self.buckets.read().await;
            if !buckets.contains_key(bucket) {
                return Err(StoreError::BucketNotFound(bucket.to_string()));
            }
        }

        let upload_id = Uuid::new_v4().to_string();
        let mut uploads = self.uploads.write().await;
        uploads.insert(
            upload_id.clone(),
            MultipartUpload {
                upload_id: upload_id.clone(),
                bucket: bucket.to_string(),
                key: key.to_string(),
                parts: std::collections::BTreeMap::new(),
                initiated_at: Utc::now(),
                metadata,
            },
        );
        Ok(upload_id)
    }

    pub async fn upload_part(
        &self,
        upload_id: &str,
        part_number: u32,
        data: Vec<u8>,
    ) -> StoreResult<String> {
        let etag = compute_etag(&data);
        let mut uploads = self.uploads.write().await;
        let upload = uploads
            .get_mut(upload_id)
            .ok_or_else(|| StoreError::UploadNotFound(upload_id.to_string()))?;
        upload.parts.insert(
            part_number,
            UploadPart {
                part_number,
                etag: etag.clone(),
                data,
            },
        );
        Ok(etag)
    }

    pub async fn complete_multipart_upload(
        &self,
        upload_id: &str,
        parts: Vec<(u32, String)>, // (part_number, etag)
    ) -> StoreResult<ObjectInfo> {
        let upload = {
            let mut uploads = self.uploads.write().await;
            uploads
                .remove(upload_id)
                .ok_or_else(|| StoreError::UploadNotFound(upload_id.to_string()))?
        };

        // Validate parts and assemble data
        let mut assembled = Vec::new();
        for (part_num, expected_etag) in &parts {
            let part = upload
                .parts
                .get(part_num)
                .ok_or(StoreError::InvalidPart)?;
            if &part.etag != expected_etag {
                return Err(StoreError::InvalidPart);
            }
            assembled.extend_from_slice(&part.data);
        }

        // Store the assembled object
        self.put_object(
            &upload.bucket,
            &upload.key,
            assembled,
            "application/octet-stream",
            upload.metadata.clone(),
            None,
        )
        .await
    }

    pub async fn abort_multipart_upload(&self, upload_id: &str) -> StoreResult<()> {
        let mut uploads = self.uploads.write().await;
        if uploads.remove(upload_id).is_none() {
            return Err(StoreError::UploadNotFound(upload_id.to_string()));
        }
        Ok(())
    }

    pub async fn list_parts(&self, upload_id: &str) -> StoreResult<Vec<PartInfo>> {
        let uploads = self.uploads.read().await;
        let upload = uploads
            .get(upload_id)
            .ok_or_else(|| StoreError::UploadNotFound(upload_id.to_string()))?;
        let parts = upload
            .parts
            .values()
            .map(|p| PartInfo {
                part_number: p.part_number,
                etag: p.etag.clone(),
                size: p.data.len() as u64,
                last_modified: upload.initiated_at,
            })
            .collect();
        Ok(parts)
    }
}
