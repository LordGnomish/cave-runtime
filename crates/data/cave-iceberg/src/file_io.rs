// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg FileIO abstraction.
//!
//! Upstream: `crates/iceberg/src/io/`
//!
//! The Iceberg spec separates "object-store IO" from the table logic
//! itself. A `FileIO` impl can return any byte stream — the table
//! reader doesn't care whether bytes come from S3, GCS, MinIO, or a
//! local disk. This module ships an in-memory implementation that
//! powers tests and unit-test fixtures. A real S3/MinIO impl lives
//! behind cave-store (cross-crate dep would land in a v0.2 milestone).

use crate::error::{Error, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait FileIo: Send + Sync {
    async fn read(&self, path: &str) -> Result<Vec<u8>>;
    async fn write(&self, path: &str, bytes: Vec<u8>) -> Result<()>;
    async fn exists(&self, path: &str) -> Result<bool>;
    async fn delete(&self, path: &str) -> Result<()>;
}

/// In-memory FileIO. Backed by an `Arc<Mutex<HashMap>>` so that
/// scan-time / commit-time reads see commit-time writes within a
/// single test process.
#[derive(Debug, Clone, Default)]
pub struct MemFileIo {
    inner: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl MemFileIo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl FileIo for MemFileIo {
    async fn read(&self, path: &str) -> Result<Vec<u8>> {
        self.inner
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| Error::NotFound(path.to_string()))
    }

    async fn write(&self, path: &str, bytes: Vec<u8>) -> Result<()> {
        self.inner.lock().unwrap().insert(path.to_string(), bytes);
        Ok(())
    }

    async fn exists(&self, path: &str) -> Result<bool> {
        Ok(self.inner.lock().unwrap().contains_key(path))
    }

    async fn delete(&self, path: &str) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .remove(path)
            .ok_or_else(|| Error::NotFound(path.to_string()))?;
        Ok(())
    }
}

/// Filesystem-backed FileIO. Resolves `file://` URIs and bare paths to
/// local disk via `std::fs`, creating parent directories on write. This
/// is the local backend that pairs with the in-memory `MemFileIo`; the
/// cloud object-store backends (S3 / GCS / ADLS) remain an explicit
/// scope_cut behind cave-store.
#[derive(Debug, Clone, Default)]
pub struct LocalFileIo;

impl LocalFileIo {
    pub fn new() -> Self {
        Self
    }

    /// Strip an optional `file://` scheme, yielding a local filesystem
    /// path.
    fn resolve(path: &str) -> &str {
        path.strip_prefix("file://").unwrap_or(path)
    }
}

#[async_trait]
impl FileIo for LocalFileIo {
    async fn read(&self, path: &str) -> Result<Vec<u8>> {
        let p = Self::resolve(path);
        std::fs::read(p).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(path.to_string()),
            _ => Error::Io(e.to_string()),
        })
    }

    async fn write(&self, path: &str, bytes: Vec<u8>) -> Result<()> {
        let p = Self::resolve(path);
        if let Some(parent) = std::path::Path::new(p).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| Error::Io(e.to_string()))?;
            }
        }
        std::fs::write(p, bytes).map_err(|e| Error::Io(e.to_string()))
    }

    async fn exists(&self, path: &str) -> Result<bool> {
        Ok(std::path::Path::new(Self::resolve(path)).exists())
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let p = Self::resolve(path);
        std::fs::remove_file(p).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(path.to_string()),
            _ => Error::Io(e.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mem_io_round_trip() {
        let io = MemFileIo::new();
        assert!(!io.exists("k").await.unwrap());
        io.write("k", b"hello".to_vec()).await.unwrap();
        assert!(io.exists("k").await.unwrap());
        let r = io.read("k").await.unwrap();
        assert_eq!(r, b"hello");
        io.delete("k").await.unwrap();
        assert!(!io.exists("k").await.unwrap());
    }

    #[tokio::test]
    async fn mem_io_read_missing_returns_not_found() {
        let io = MemFileIo::new();
        let r = io.read("nope").await;
        assert!(matches!(r, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn mem_io_delete_missing_returns_not_found() {
        let io = MemFileIo::new();
        let r = io.delete("nope").await;
        assert!(matches!(r, Err(Error::NotFound(_))));
    }
}
