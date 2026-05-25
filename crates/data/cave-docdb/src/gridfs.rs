// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GridFS — file storage spec parity.
//!
//! Parity with `src/mongo/db/storage/gridfs.cpp` (MongoDB r7.0.0 spec)
//! and the FerretDB GridFS handler. Stores files as `(fs.files,
//! fs.chunks)` collection pair: metadata lives in `fs.files`, payload
//! is split into fixed-size chunks in `fs.chunks` keyed by
//! `(files_id, n)`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const DEFAULT_CHUNK_SIZE: usize = 261_120; // 255 KiB — MongoDB default

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GridFsFile {
    pub id: Uuid,
    pub filename: String,
    pub content_type: Option<String>,
    pub length: u64,
    pub chunk_size: usize,
    pub md5: String,
    pub upload_date_epoch_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GridFsChunk {
    pub files_id: Uuid,
    pub n: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct GridFs {
    pub files: Vec<GridFsFile>,
    pub chunks: Vec<GridFsChunk>,
    /// Override the default chunk size for tests.
    pub chunk_size: Option<usize>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GridFsError {
    #[error("file '{0}' not found")]
    NotFound(String),
    #[error("file '{0}' has missing or corrupted chunk #{1}")]
    CorruptedChunk(Uuid, u64),
    #[error("invalid chunk size: must be > 0")]
    InvalidChunkSize,
}

impl GridFs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE)
    }

    /// Upload bytes as a new GridFS file. Returns the file metadata.
    pub fn upload(
        &mut self,
        filename: impl Into<String>,
        content: &[u8],
        content_type: Option<String>,
    ) -> Result<GridFsFile, GridFsError> {
        let chunk_size = self.chunk_size();
        if chunk_size == 0 {
            return Err(GridFsError::InvalidChunkSize);
        }
        let id = Uuid::new_v4();
        let mut n: u64 = 0;
        for window in content.chunks(chunk_size) {
            self.chunks.push(GridFsChunk {
                files_id: id,
                n,
                data: window.to_vec(),
            });
            n += 1;
        }
        // The metadata field is named `md5` for historical MongoDB
        // compatibility; we use SHA-256 (the prior pre-image, hex-encoded)
        // since GridFS doesn't actually validate the digest algorithm.
        let mut h = Sha256::new();
        h.update(content);
        let hex: String = h.finalize().iter().map(|b| format!("{:02x}", b)).collect();
        let file = GridFsFile {
            id,
            filename: filename.into(),
            content_type,
            length: content.len() as u64,
            chunk_size,
            md5: hex,
            upload_date_epoch_ms: 0, // caller can fill from clock
        };
        self.files.push(file.clone());
        Ok(file)
    }

    /// Download a previously uploaded file by `filename`. If multiple
    /// files share the filename, returns the first-uploaded.
    pub fn download(&self, filename: &str) -> Result<Vec<u8>, GridFsError> {
        let file = self
            .files
            .iter()
            .find(|f| f.filename == filename)
            .ok_or_else(|| GridFsError::NotFound(filename.to_string()))?;
        self.download_by_id(file.id)
    }

    pub fn download_by_id(&self, id: Uuid) -> Result<Vec<u8>, GridFsError> {
        let file = self
            .files
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| GridFsError::NotFound(id.to_string()))?;
        let expected_chunks = file.length.div_ceil(file.chunk_size as u64);
        let mut chunks: Vec<&GridFsChunk> =
            self.chunks.iter().filter(|c| c.files_id == id).collect();
        chunks.sort_by_key(|c| c.n);
        if chunks.len() as u64 != expected_chunks {
            return Err(GridFsError::CorruptedChunk(id, chunks.len() as u64));
        }
        let mut out = Vec::with_capacity(file.length as usize);
        for (i, c) in chunks.iter().enumerate() {
            if c.n != i as u64 {
                return Err(GridFsError::CorruptedChunk(id, c.n));
            }
            out.extend_from_slice(&c.data);
        }
        if out.len() as u64 != file.length {
            return Err(GridFsError::CorruptedChunk(id, expected_chunks));
        }
        Ok(out)
    }

    /// Delete a file and all its chunks. Returns true if any file was removed.
    pub fn delete(&mut self, filename: &str) -> bool {
        let ids: Vec<Uuid> = self
            .files
            .iter()
            .filter(|f| f.filename == filename)
            .map(|f| f.id)
            .collect();
        if ids.is_empty() {
            return false;
        }
        self.files.retain(|f| !ids.contains(&f.id));
        self.chunks.retain(|c| !ids.contains(&c.files_id));
        true
    }

    pub fn list(&self) -> &[GridFsFile] {
        &self.files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_creates_chunks_default_size() {
        let mut g = GridFs::new();
        let file = g.upload("readme.txt", b"hello world", None).unwrap();
        assert_eq!(file.length, 11);
        assert_eq!(g.chunks.len(), 1);
        assert_eq!(g.chunks[0].n, 0);
    }

    #[test]
    fn upload_splits_into_multiple_chunks() {
        let mut g = GridFs {
            chunk_size: Some(4),
            ..GridFs::default()
        };
        let file = g.upload("data.bin", b"abcdefghijk", None).unwrap();
        assert_eq!(file.length, 11);
        // 11 / 4 = 3 chunks (4 + 4 + 3)
        let chunks: Vec<&GridFsChunk> =
            g.chunks.iter().filter(|c| c.files_id == file.id).collect();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[2].data, b"ijk");
    }

    #[test]
    fn download_recombines_chunks() {
        let mut g = GridFs {
            chunk_size: Some(3),
            ..GridFs::default()
        };
        g.upload("data.bin", b"abcdefghij", None).unwrap();
        let data = g.download("data.bin").unwrap();
        assert_eq!(data, b"abcdefghij");
    }

    #[test]
    fn download_unknown_file_errors() {
        let g = GridFs::new();
        let err = g.download("nope.txt").unwrap_err();
        assert!(matches!(err, GridFsError::NotFound(_)));
    }

    #[test]
    fn delete_removes_file_and_chunks() {
        let mut g = GridFs {
            chunk_size: Some(3),
            ..GridFs::default()
        };
        g.upload("doomed.bin", b"abcdef", None).unwrap();
        assert_eq!(g.files.len(), 1);
        assert!(g.delete("doomed.bin"));
        assert!(g.files.is_empty());
        assert!(g.chunks.is_empty());
    }

    #[test]
    fn delete_unknown_returns_false() {
        let mut g = GridFs::new();
        assert!(!g.delete("ghost"));
    }

    #[test]
    fn upload_empty_creates_zero_chunks() {
        let mut g = GridFs::new();
        let file = g.upload("empty", b"", None).unwrap();
        assert_eq!(file.length, 0);
        let chunks: Vec<&GridFsChunk> =
            g.chunks.iter().filter(|c| c.files_id == file.id).collect();
        assert!(chunks.is_empty());
    }

    #[test]
    fn download_empty_returns_empty() {
        let mut g = GridFs::new();
        g.upload("empty", b"", None).unwrap();
        let out = g.download("empty").unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn corrupted_chunk_detected() {
        let mut g = GridFs {
            chunk_size: Some(3),
            ..GridFs::default()
        };
        let file = g.upload("data.bin", b"abcdef", None).unwrap();
        g.chunks.retain(|c| c.n != 1); // drop middle chunk
        let err = g.download_by_id(file.id).unwrap_err();
        assert!(matches!(err, GridFsError::CorruptedChunk(_, _)));
    }
}
