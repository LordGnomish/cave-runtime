// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: pulp/pulpcore@0f991c2fa2bf6c8635e8a2de064ef04dacbbcf4f pulpcore/app/tasks/upload.py + pulpcore/app/models/upload.py
//! Chunked upload API — Pulp v3 upload workflow.
//!
//! Upload → finalize → create artifact → create content → add to repo.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

// ─── Upload session ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub pulp_created: DateTime<Utc>,
    pub offset: u64,
    pub size: u64,
    pub completed: bool,
    pub artifact: Option<String>,
}

impl Upload {
    pub fn new(size: u64) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/uploads/{}/", id),
            pulp_id: id,
            pulp_created: Utc::now(),
            offset: 0,
            size,
            completed: false,
            artifact: None,
        }
    }

    /// Accept a chunk and advance the offset.
    pub fn accept_chunk(&mut self, chunk_offset: u64, chunk_size: u64) -> Result<(), UploadError> {
        if self.completed {
            return Err(UploadError::AlreadyCompleted);
        }
        if chunk_offset != self.offset {
            return Err(UploadError::OutOfOrder {
                expected: self.offset,
                got: chunk_offset,
            });
        }
        if self.offset + chunk_size > self.size {
            return Err(UploadError::ExceedsSize {
                upload_size: self.size,
                would_be: self.offset + chunk_size,
            });
        }
        self.offset += chunk_size;
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        self.offset >= self.size
    }

    pub fn finalize(&mut self, artifact_href: impl Into<String>) {
        self.completed = true;
        self.artifact = Some(artifact_href.into());
    }

    pub fn progress_pct(&self) -> f64 {
        if self.size == 0 { return 100.0; }
        (self.offset as f64 / self.size as f64) * 100.0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("Upload already completed")]
    AlreadyCompleted,
    #[error("Chunk out of order: expected offset {expected}, got {got}")]
    OutOfOrder { expected: u64, got: u64 },
    #[error("Chunk would exceed upload size {upload_size} (would be {would_be})")]
    ExceedsSize { upload_size: u64, would_be: u64 },
    #[error("Upload {0} not found")]
    NotFound(Uuid),
    #[error("Upload not complete: {offset}/{size} bytes received")]
    Incomplete { offset: u64, size: u64 },
}

// ─── Upload session registry ─────────────────────────────────────────────────

pub struct UploadRegistry {
    uploads: Mutex<HashMap<Uuid, Upload>>,
}

impl UploadRegistry {
    pub fn new() -> Self {
        Self { uploads: Mutex::new(HashMap::new()) }
    }

    pub fn create(&self, size: u64) -> Upload {
        let upload = Upload::new(size);
        self.uploads.lock().unwrap().insert(upload.pulp_id, upload.clone());
        upload
    }

    pub fn get(&self, id: &Uuid) -> Option<Upload> {
        self.uploads.lock().unwrap().get(id).cloned()
    }

    pub fn apply_chunk(&self, id: &Uuid, offset: u64, size: u64) -> Result<Upload, UploadError> {
        let mut uploads = self.uploads.lock().unwrap();
        let upload = uploads.get_mut(id).ok_or(UploadError::NotFound(*id))?;
        upload.accept_chunk(offset, size)?;
        Ok(upload.clone())
    }

    pub fn finalize(&self, id: &Uuid, artifact_href: impl Into<String>) -> Result<Upload, UploadError> {
        let mut uploads = self.uploads.lock().unwrap();
        let upload = uploads.get_mut(id).ok_or(UploadError::NotFound(*id))?;
        if !upload.is_complete() {
            return Err(UploadError::Incomplete { offset: upload.offset, size: upload.size });
        }
        upload.finalize(artifact_href);
        Ok(upload.clone())
    }

    pub fn delete(&self, id: &Uuid) -> bool {
        self.uploads.lock().unwrap().remove(id).is_some()
    }

    pub fn list(&self) -> Vec<Upload> {
        self.uploads.lock().unwrap().values().cloned().collect()
    }
}

// ─── Upload request ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadChunkRequest {
    pub upload_id: Uuid,
    pub offset: u64,
    pub content_range: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizeUploadRequest {
    pub sha256: String,
}

/// Parse a Content-Range header: "bytes 0-1023/2048"
pub fn parse_content_range(header: &str) -> Option<(u64, u64, u64)> {
    let rest = header.trim_start_matches("bytes ");
    let (range_part, total_part) = rest.split_once('/')?;
    let (start_str, end_str) = range_part.split_once('-')?;
    let start: u64 = start_str.parse().ok()?;
    let end: u64 = end_str.parse().ok()?;
    let total: u64 = total_part.parse().ok()?;
    Some((start, end, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_session_sequential_chunks() {
        let mut upload = Upload::new(1024);
        assert_eq!(upload.offset, 0);

        upload.accept_chunk(0, 512).unwrap();
        assert_eq!(upload.offset, 512);
        assert!(!upload.is_complete());

        upload.accept_chunk(512, 512).unwrap();
        assert_eq!(upload.offset, 1024);
        assert!(upload.is_complete());
    }

    #[test]
    fn upload_chunk_out_of_order() {
        let mut upload = Upload::new(1024);
        let err = upload.accept_chunk(512, 512).unwrap_err();
        assert!(matches!(err, UploadError::OutOfOrder { expected: 0, got: 512 }));
    }

    #[test]
    fn upload_chunk_exceeds_size() {
        let mut upload = Upload::new(1024);
        let err = upload.accept_chunk(0, 2048).unwrap_err();
        assert!(matches!(err, UploadError::ExceedsSize { upload_size: 1024, .. }));
    }

    #[test]
    fn upload_already_completed() {
        let mut upload = Upload::new(512);
        upload.accept_chunk(0, 512).unwrap();
        upload.finalize("/pulp/api/v3/artifacts/abc/");
        let err = upload.accept_chunk(0, 512).unwrap_err();
        assert!(matches!(err, UploadError::AlreadyCompleted));
    }

    #[test]
    fn upload_progress_percentage() {
        let mut upload = Upload::new(1000);
        upload.accept_chunk(0, 250).unwrap();
        assert!((upload.progress_pct() - 25.0).abs() < 0.01);
    }

    #[test]
    fn upload_registry_workflow() {
        let registry = UploadRegistry::new();
        let upload = registry.create(1024);
        let id = upload.pulp_id;

        registry.apply_chunk(&id, 0, 512).unwrap();
        registry.apply_chunk(&id, 512, 512).unwrap();

        let finalized = registry.finalize(&id, "/pulp/api/v3/artifacts/xyz/").unwrap();
        assert!(finalized.completed);
        assert!(finalized.artifact.is_some());
    }

    #[test]
    fn upload_registry_finalize_incomplete() {
        let registry = UploadRegistry::new();
        let upload = registry.create(1024);
        let id = upload.pulp_id;
        registry.apply_chunk(&id, 0, 512).unwrap();
        let err = registry.finalize(&id, "/artifact/").unwrap_err();
        assert!(matches!(err, UploadError::Incomplete { .. }));
    }

    #[test]
    fn parse_content_range_valid() {
        let (start, end, total) = parse_content_range("bytes 0-1023/2048").unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 1023);
        assert_eq!(total, 2048);
    }

    #[test]
    fn parse_content_range_invalid() {
        assert!(parse_content_range("invalid").is_none());
        assert!(parse_content_range("bytes 0-abc/2048").is_none());
    }
}
